//! Characterization gate for `Quake/host.c`. Rust migration Phase 7, M8
//! (task T8.1); the port itself is a later task, so there is no Rust side to
//! diff against yet and every test below pins the C reference against an
//! independently written expectation.
//!
//! The oracle side is `stubs/host_ref.c`, which composes `Quake/host.c` into
//! its own translation unit behind a per-TU rename layer (see that file's
//! header for why the prelude cannot do the renaming here) and exports one
//! `ctest_host_*` driver per subject. Composition also makes host.c's
//! file-static cvar callbacks `Max_Fps_f` (host.c:131) and `Phys_Ticrate_f`
//! (host.c:162) reachable.
//!
//! `Host_FilterTime` (host.c:773) is the frame clock, so nothing here uses
//! `==` on a float: every time value is compared through `to_bits`, and the
//! expectation is a transcription of the C body that reproduces its exact
//! type ladder -- `realtime` and `oldrealtime` are `double`, but
//! `delta_since_last_frame`, `min_frame_time` and `maxfps` are `float`, and
//! `CLAMP` (q_minmax.h:64) promotes to `double` whenever a `double` literal
//! is one of its operands. Getting that ladder wrong changes the result, so
//! the transcription is a real second opinion, not a restatement.
//!
//! ADR-010: only plain arithmetic operators appear below, never `f32`/`f64`
//! math methods.
//!
//! Known gaps, deliberately not covered here:
//!  - `Host_Init`/`Host_Shutdown`/`Host_Frame`/`Host_ServerFrame` are not
//!    driven; their bodies reach renderer, sound and network subsystems that
//!    `host_ref.c` can only supply as aborting link doubles.
//!  - `Host_Error`/`Host_EndGame`/`Host_Guard`/`Host_Reraise` stay C until
//!    Phase 9 per ADR-009 and are exercised only indirectly, by the harness's
//!    own trap machinery in `stubs/stubs.c`.
//!  - `Host_WriteConfiguration` is driven with an empty cvar list, so it pins
//!    the guard ladder, the `Key_WriteBindings` call and the two trailing
//!    commands, not `Cvar_WriteVariables`' output (that is `cvar.c`'s gate).

use core::ffi::{c_char, c_float, c_int, c_uint, CStr};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

// protocol.h:253 / protocol.h:254
const SVC_PRINT: u8 = 8;
const SVC_STUFFTEXT: u8 = 9;
// quakedef.h:214
const MAX_SCOREBOARD: c_int = 16;
// quakedef.h:70
const HOST_NETITERVAL_FREQ: f32 = 71.9990;
// quakedef.h:68
const MAX_PHYSICS_FREQ: f32 = 72.0;
// client.h:104-106: ca_dedicated, ca_disconnected, ca_connected
const CA_DISCONNECTED: c_int = 1;

extern "C" {
    fn ctest_host_reset_time(realtime: f64, oldrealtime: f64);
    fn ctest_host_set_maxfps(value: c_float);
    fn ctest_host_set_timescale(value: c_float);
    fn ctest_host_set_framerate(value: c_float);
    fn ctest_host_set_demo(demoplayback: c_int, demospeed: c_float, timedemo: c_int);
    fn ctest_host_filter_time(time: c_float) -> c_int;
    fn ctest_host_get_realtime() -> f64;
    fn ctest_host_get_oldrealtime() -> f64;
    fn ctest_host_get_frametime() -> f64;
    fn ctest_host_get_rawframetime() -> f64;
    fn ctest_host_get_netinterval() -> c_float;
    fn ctest_host_set_netinterval(value: c_float);
    fn ctest_host_max_fps_f(value: c_float);
    fn ctest_host_phys_ticrate_f(value: c_float);
    fn ctest_host_set_phys_max_ticrate(value: c_float);
    fn ctest_host_get_phys_max_ticrate() -> c_float;
    fn ctest_host_callback_notify(var: *mut CvarMirror);
    fn ctest_host_client_commands(text: *const c_char);
    fn ctest_host_sv_client_printf(text: *const c_char);
    fn ctest_host_sv_broadcast_printf(text: *const c_char);
    fn ctest_host_set_host_client(index: c_int);
    fn ctest_host_find_max_clients();
    fn ctest_host_write_configuration();
    fn ctest_host_set_initialized(value: c_int);
    fn ctest_host_sdl_delay_calls() -> c_int;
    fn ctest_host_sdl_delay_last_ms() -> c_uint;
    fn ctest_host_sdl_delay_reset();
    fn ctest_host_reset_clients(maxclients: c_int);
    fn ctest_host_set_client_state(index: c_int, active: c_int, spawned: c_int);
    fn ctest_host_client_msg_len(index: c_int) -> c_int;
    fn ctest_host_client_msg_byte(index: c_int, offset: c_int) -> c_int;
    fn ctest_host_set_sv_active(value: c_int);
    fn ctest_host_get_maxclients() -> c_int;
    fn ctest_host_set_parms(errstate: c_int);
    fn ctest_host_key_write_bindings_calls() -> c_int;
    fn ctest_host_set_cmdline(a0: *const c_char, a1: *const c_char, a2: *const c_char);
    fn ctest_host_get_deathmatch() -> c_float;
    fn ctest_host_get_deathmatch_string() -> *const c_char;
    fn ctest_host_register_deathmatch();
    fn ctest_host_get_cls_state() -> c_int;
    fn ctest_host_set_gamedir(dir: *const c_char);

    fn ctest_clear_con_log();
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;
}

/// ADR-011 `#[repr(C)]` mirror of `cvar_t` (`Quake/cvar.h`). Only `name` and
/// `string` are dereferenced by `Host_Callback_Notify` (host.c:417), but the
/// whole struct is mirrored so the pointer handed to C is a valid `cvar_t`.
#[repr(C)]
struct CvarMirror {
    name: *const c_char,
    string: *const c_char,
    flags: c_uint,
    value: c_float,
    default_string: *const c_char,
    callback: Option<extern "C" fn(*mut CvarMirror)>,
    next: *mut CvarMirror,
}

fn con_log() -> Vec<String> {
    let mut out = Vec::new();
    // SAFETY: ADR-004. Reads back the console capture log stubs.c owns; TEST_LOCK is held by the caller.
    unsafe {
        for i in 0..ctest_con_log_len() {
            out.push(
                CStr::from_ptr(ctest_con_log_get(i))
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    out
}

fn deathmatch_string() -> String {
    // SAFETY: ADR-004. Reads the deathmatch cvar's string, static storage cvar.c owns.
    unsafe {
        CStr::from_ptr(ctest_host_get_deathmatch_string())
            .to_string_lossy()
            .into_owned()
    }
}

fn client_msg(index: c_int) -> Vec<u8> {
    // SAFETY: ADR-004. Reads the client message buffers the fixture in stubs.c owns.
    unsafe {
        (0..ctest_host_client_msg_len(index))
            .map(|o| ctest_host_client_msg_byte(index, o) as u8)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Host_FilterTime (host.c:773), transcribed.

fn clamp_f64(minval: f64, val: f64, maxval: f64) -> f64 {
    if val < minval {
        minval
    } else if val > maxval {
        maxval
    } else {
        val
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FilterOut {
    ret: c_int,
    realtime: u64,
    oldrealtime: u64,
    frametime: u64,
    rawframetime: u64,
    delay_calls: c_int,
}

#[derive(Debug, Clone, Copy)]
struct FilterIn {
    realtime: f64,
    oldrealtime: f64,
    time: f32,
    maxfps: f32,
    timescale: f32,
    framerate: f32,
    demoplayback: bool,
    demospeed: f32,
    timedemo: bool,
}

impl Default for FilterIn {
    fn default() -> Self {
        FilterIn {
            realtime: 0.0,
            oldrealtime: 0.0,
            time: 0.0,
            maxfps: 0.0,
            timescale: 0.0,
            framerate: 0.0,
            demoplayback: false,
            demospeed: 1.0,
            timedemo: false,
        }
    }
}

fn expected_filter_time(i: FilterIn) -> FilterOut {
    let realtime: f64 = i.realtime + i.time as f64;
    // host.c:777 declares `float delta_since_last_frame`, so the double
    // difference is narrowed here and every later comparison is an f32 one.
    let delta: f32 = (realtime - i.oldrealtime) as f32;
    let mut delay_calls: c_int = 0;

    if i.maxfps != 0.0 {
        // CLAMP(10.0, <float>, 1000.0) selects clamp_double, then narrows.
        let maxfps: f32 = clamp_f64(10.0, i.maxfps as f64, 1000.0) as f32;
        let min_frame_time: f32 = 1.0f32 / maxfps;
        if (min_frame_time - delta) > (2.0f32 / 1000.0f32) {
            delay_calls = 1;
        }
        if !i.timedemo && delta < min_frame_time {
            return FilterOut {
                ret: 0,
                realtime: realtime.to_bits(),
                oldrealtime: i.oldrealtime.to_bits(),
                frametime: 0.0f64.to_bits(),
                rawframetime: 0.0f64.to_bits(),
                delay_calls,
            };
        }
    }

    let rawframetime: f64 = delta as f64;
    let mut frametime: f64 = rawframetime;

    if i.demoplayback && i.demospeed != 1.0 && i.demospeed > 0.0 {
        frametime *= i.demospeed as f64;
    } else if i.timescale > 0.0 {
        frametime *= i.timescale as f64;
    } else if i.framerate > 0.0 {
        frametime = i.framerate as f64;
    } else if i.maxfps != 0.0 {
        frametime = clamp_f64(0.0001, frametime, 0.1);
    }

    FilterOut {
        ret: 1,
        realtime: realtime.to_bits(),
        oldrealtime: realtime.to_bits(),
        frametime: frametime.to_bits(),
        rawframetime: rawframetime.to_bits(),
        delay_calls,
    }
}

fn run_filter_time(i: FilterIn) -> FilterOut {
    // SAFETY: ADR-004. Seeds host.c's file-scope timing state and calls Host_FilterTime over it; TEST_LOCK is held by the caller.
    unsafe {
        ctest_host_reset_time(i.realtime, i.oldrealtime);
        ctest_host_set_maxfps(i.maxfps);
        ctest_host_set_timescale(i.timescale);
        ctest_host_set_framerate(i.framerate);
        ctest_host_set_demo(i.demoplayback as c_int, i.demospeed, i.timedemo as c_int);
        ctest_host_sdl_delay_reset();
        let ret = ctest_host_filter_time(i.time);
        FilterOut {
            ret,
            realtime: ctest_host_get_realtime().to_bits(),
            oldrealtime: ctest_host_get_oldrealtime().to_bits(),
            frametime: ctest_host_get_frametime().to_bits(),
            rawframetime: ctest_host_get_rawframetime().to_bits(),
            delay_calls: ctest_host_sdl_delay_calls(),
        }
    }
}

fn check(label: &str, i: FilterIn) -> FilterOut {
    let got = run_filter_time(i);
    let want = expected_filter_time(i);
    assert_eq!(got, want, "{label}: input {i:?}");
    got
}

#[test]
fn filter_time_uncapped_passes_the_raw_delta_through() {
    let _g = lock();
    let out = check(
        "uncapped",
        FilterIn {
            realtime: 10.0,
            oldrealtime: 9.75,
            time: 0.25,
            ..FilterIn::default()
        },
    );
    assert_eq!(out.ret, 1);
    // maxfps == 0 skips both the sleep branch and the CLAMP, so frametime is
    // exactly the narrowed delta and nothing else.
    assert_eq!(out.frametime, out.rawframetime);
    assert_eq!(out.frametime, (0.5f32 as f64).to_bits());
    assert_eq!(out.delay_calls, 0);
}

#[test]
fn filter_time_rejects_a_frame_that_is_too_early() {
    let _g = lock();
    let out = check(
        "too early",
        FilterIn {
            realtime: 100.0,
            oldrealtime: 100.0,
            time: 0.001,
            maxfps: 72.0,
            ..FilterIn::default()
        },
    );
    assert_eq!(out.ret, 0);
    // host.c:786 advances realtime before the rejection but leaves
    // oldrealtime alone, which is what makes the next call see the sum.
    // 0.001f32 is not 0.001, and host.c:775 widens the float argument into
    // the double accumulator, so the sum is the widened one.
    assert_eq!(out.realtime, (100.0f64 + 0.001f32 as f64).to_bits());
    assert_eq!(out.oldrealtime, 100.0f64.to_bits());
    assert_eq!(out.frametime, 0.0f64.to_bits());
    // 1/72 - 0.001 is well over 2ms, so host.c:796 sleeps.
    assert_eq!(out.delay_calls, 1);
    // SAFETY: ADR-004. Reads the SDL_Delay double's counter (stubs.c).
    assert_eq!(unsafe { ctest_host_sdl_delay_last_ms() }, 1);
}

#[test]
fn filter_time_skips_the_sleep_inside_the_two_millisecond_window() {
    let _g = lock();
    // 1/72 == 0.013888...; a 0.0125 delta leaves 1.39ms, under the 2ms
    // threshold at host.c:796, yet still short of a full frame.
    let out = check(
        "inside window",
        FilterIn {
            time: 0.0125,
            maxfps: 72.0,
            ..FilterIn::default()
        },
    );
    assert_eq!(out.ret, 0);
    assert_eq!(out.delay_calls, 0);
}

#[test]
fn filter_time_timedemo_ignores_the_frame_cap_but_not_the_sleep() {
    let _g = lock();
    let out = check(
        "timedemo",
        FilterIn {
            time: 0.001,
            maxfps: 72.0,
            timedemo: true,
            ..FilterIn::default()
        },
    );
    // host.c:799 gates only the early return on cls.timedemo; the sleep at
    // host.c:796 still happens.
    assert_eq!(out.ret, 1);
    assert_eq!(out.delay_calls, 1);
    // 0.001 is above the 0.0001 floor, so the CLAMP leaves it alone.
    assert_eq!(out.frametime, (0.001f32 as f64).to_bits());
}

#[test]
fn filter_time_clamps_a_long_frame_to_a_tenth_of_a_second() {
    let _g = lock();
    let out = check(
        "long frame",
        FilterIn {
            time: 3.0,
            maxfps: 72.0,
            ..FilterIn::default()
        },
    );
    assert_eq!(out.ret, 1);
    assert_eq!(out.frametime, 0.1f64.to_bits());
    // rawframetime keeps the pre-clamp value, which is the whole reason it
    // exists separately (host.c:802).
    assert_eq!(out.rawframetime, (3.0f32 as f64).to_bits());
}

#[test]
fn filter_time_clamps_a_short_accepted_frame_up_to_the_floor() {
    let _g = lock();
    let out = check(
        "short frame",
        FilterIn {
            time: 0.00001,
            maxfps: 72.0,
            timedemo: true,
            ..FilterIn::default()
        },
    );
    assert_eq!(out.ret, 1);
    assert_eq!(out.frametime, 0.0001f64.to_bits());
    assert_eq!(out.rawframetime, (0.00001f32 as f64).to_bits());
}

#[test]
fn filter_time_maxfps_cvar_is_clamped_at_both_ends() {
    let _g = lock();
    // host.c:790 clamps the cvar into [10, 1000] before deriving the minimum
    // frame time, so 1 behaves as 10 and 5000 behaves as 1000.
    let slow = check(
        "maxfps 1",
        FilterIn {
            time: 0.05,
            maxfps: 1.0,
            ..FilterIn::default()
        },
    );
    let ten = check(
        "maxfps 10",
        FilterIn {
            time: 0.05,
            maxfps: 10.0,
            ..FilterIn::default()
        },
    );
    assert_eq!(slow, ten);
    // ...and the bound really is 10, not just "some clamp": at maxfps 1 a
    // 0.095s delta is still short of the 0.1s frame time it implies.
    let below = check(
        "maxfps 1, just under 1/10",
        FilterIn {
            time: 0.095,
            maxfps: 1.0,
            ..FilterIn::default()
        },
    );
    assert_eq!(below.ret, 0);

    // 1/1000 == 0.001; a 0.0009 delta is rejected at 1000fps, and the sleep
    // is skipped because the shortfall is only 0.1ms.
    let fast = check(
        "maxfps 5000",
        FilterIn {
            time: 0.0009,
            maxfps: 5000.0,
            ..FilterIn::default()
        },
    );
    let thousand = check(
        "maxfps 1000",
        FilterIn {
            time: 0.0009,
            maxfps: 1000.0,
            ..FilterIn::default()
        },
    );
    assert_eq!(fast, thousand);
    assert_eq!(fast.ret, 0);
    assert_eq!(fast.delay_calls, 0);
    // ...and the upper bound is exactly 1000: a 1ms delta is a whole frame
    // there, so host.c:799 accepts it instead of holding it back.
    let exact = check(
        "maxfps 5000, exactly 1/1000",
        FilterIn {
            time: 0.001,
            maxfps: 5000.0,
            ..FilterIn::default()
        },
    );
    assert_eq!(exact.ret, 1);
}

#[test]
fn filter_time_demospeed_scales_the_frame() {
    let _g = lock();
    let out = check(
        "demospeed",
        FilterIn {
            time: 0.25,
            demoplayback: true,
            demospeed: 0.5,
            // both later branches are armed, to prove demospeed wins
            timescale: 4.0,
            framerate: 8.0,
            ..FilterIn::default()
        },
    );
    assert_eq!(out.ret, 1);
    assert_eq!(
        out.frametime,
        ((0.25f32 as f64) * (0.5f32 as f64)).to_bits()
    );
}

#[test]
fn filter_time_demospeed_of_one_falls_through_to_timescale() {
    let _g = lock();
    // host.c:805 requires demospeed != 1 AND > 0, so a normal-speed demo
    // takes the timescale branch instead.
    let out = check(
        "demospeed 1",
        FilterIn {
            time: 0.25,
            demoplayback: true,
            demospeed: 1.0,
            timescale: 4.0,
            ..FilterIn::default()
        },
    );
    assert_eq!(
        out.frametime,
        ((0.25f32 as f64) * (4.0f32 as f64)).to_bits()
    );

    let neg = check(
        "demospeed -1",
        FilterIn {
            time: 0.25,
            demoplayback: true,
            demospeed: -1.0,
            timescale: 4.0,
            ..FilterIn::default()
        },
    );
    assert_eq!(neg.frametime, out.frametime);
}

#[test]
fn filter_time_framerate_replaces_the_frame_entirely() {
    let _g = lock();
    // host.c:811 assigns rather than scales, and it beats the maxfps CLAMP
    // even for a value outside [0.0001, 0.1].
    let out = check(
        "framerate",
        FilterIn {
            time: 0.25,
            maxfps: 72.0,
            framerate: 0.5,
            ..FilterIn::default()
        },
    );
    assert_eq!(out.ret, 1);
    assert_eq!(out.frametime, (0.5f32 as f64).to_bits());
    assert_eq!(out.rawframetime, (0.25f32 as f64).to_bits());
}

#[test]
fn filter_time_accumulates_across_calls() {
    let _g = lock();
    // Two accepted frames in a row: the second must see oldrealtime moved up
    // to the first frame's realtime (host.c:803).
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        ctest_host_reset_time(0.0, 0.0);
        ctest_host_set_maxfps(0.0);
        ctest_host_set_timescale(0.0);
        ctest_host_set_framerate(0.0);
        ctest_host_set_demo(0, 1.0, 0);
        assert_eq!(ctest_host_filter_time(0.25), 1);
        assert_eq!(ctest_host_get_realtime().to_bits(), 0.25f64.to_bits());
        assert_eq!(ctest_host_get_oldrealtime().to_bits(), 0.25f64.to_bits());
        assert_eq!(ctest_host_filter_time(0.5), 1);
        assert_eq!(ctest_host_get_realtime().to_bits(), 0.75f64.to_bits());
        assert_eq!(
            ctest_host_get_frametime().to_bits(),
            (0.5f32 as f64).to_bits()
        );
    }
}

// ---------------------------------------------------------------------------
// Max_Fps_f (host.c:131) / Phys_Ticrate_f (host.c:162)

fn isolation_interval() -> u32 {
    ((1.0f64 / HOST_NETITERVAL_FREQ as f64) as f32).to_bits()
}

#[test]
fn max_fps_outside_the_physics_frequency_enables_isolation() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        ctest_host_set_phys_max_ticrate(0.0);

        ctest_host_set_netinterval(0.0);
        ctest_host_max_fps_f(200.0);
        assert_eq!(ctest_host_get_netinterval().to_bits(), isolation_interval());

        // zero and negative take the same branch (host.c:139)
        ctest_host_set_netinterval(0.0);
        ctest_host_max_fps_f(0.0);
        assert_eq!(ctest_host_get_netinterval().to_bits(), isolation_interval());
        ctest_host_set_netinterval(0.0);
        ctest_host_max_fps_f(-5.0);
        assert_eq!(ctest_host_get_netinterval().to_bits(), isolation_interval());
    }
}

#[test]
fn max_fps_announces_the_isolation_transition_only_once() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        ctest_host_set_phys_max_ticrate(0.0);
        ctest_host_set_netinterval(0.0);
        ctest_clear_con_log();
        ctest_host_max_fps_f(200.0);
        assert_eq!(
            con_log(),
            vec!["[con] Using renderer/network isolation.\n".to_string()]
        );
        ctest_clear_con_log();
        ctest_host_max_fps_f(200.0);
        assert!(con_log().is_empty(), "second call must stay quiet");
    }
}

#[test]
fn max_fps_inside_the_physics_frequency_disables_isolation() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        ctest_host_set_phys_max_ticrate(0.0);
        ctest_host_set_netinterval(1.0);
        ctest_clear_con_log();
        ctest_host_max_fps_f(MAX_PHYSICS_FREQ);
        assert_eq!(ctest_host_get_netinterval().to_bits(), 0.0f32.to_bits());
        // host.c:151 announces the transition, gated on the previous value
        // having been non-zero; the boundary value 72 is NOT "above 72", so
        // there is no physics warning.
        assert_eq!(
            con_log(),
            vec!["[con] Disabling renderer/network isolation.\n".to_string()]
        );
        ctest_clear_con_log();
        ctest_host_max_fps_f(60.0);
        assert_eq!(ctest_host_get_netinterval().to_bits(), 0.0f32.to_bits());
        assert!(con_log().is_empty());
    }
}

#[test]
fn max_fps_defers_to_phys_max_ticrate_when_it_is_set() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        // host.c:133: a positive host_phys_max_ticrate overrides the cvar
        // entirely, so a 200fps setting still yields the 50Hz interval.
        ctest_host_set_phys_max_ticrate(50.0);
        ctest_host_set_netinterval(0.0);
        ctest_host_max_fps_f(200.0);
        assert_eq!(
            ctest_host_get_netinterval().to_bits(),
            ((1.0f64 / 50.0f64) as f32).to_bits()
        );
        ctest_host_set_phys_max_ticrate(0.0);
    }
}

#[test]
fn phys_ticrate_clamps_into_the_physics_frequency_and_writes_back() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        ctest_host_set_netinterval(0.0);
        ctest_host_phys_ticrate_f(500.0);
        // host.c:170 mutates the cvar's value in place, not just the interval
        assert_eq!(
            ctest_host_get_phys_max_ticrate().to_bits(),
            MAX_PHYSICS_FREQ.to_bits()
        );
        assert_eq!(
            ctest_host_get_netinterval().to_bits(),
            ((1.0f64 / MAX_PHYSICS_FREQ as f64) as f32).to_bits()
        );
        ctest_host_set_phys_max_ticrate(0.0);
    }
}

#[test]
fn phys_ticrate_zero_falls_back_to_the_max_fps_policy() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        // With host_maxfps inside the physics frequency the fallback must
        // land on "isolation off", proving the delegation at host.c:178.
        ctest_host_set_phys_max_ticrate(0.0);
        ctest_host_set_maxfps(60.0);
        ctest_host_set_netinterval(1.0);
        ctest_host_phys_ticrate_f(0.0);
        assert_eq!(ctest_host_get_netinterval().to_bits(), 0.0f32.to_bits());

        ctest_host_set_phys_max_ticrate(0.0);
        ctest_host_set_maxfps(200.0);
        ctest_host_set_netinterval(0.0);
        ctest_host_phys_ticrate_f(0.0);
        assert_eq!(ctest_host_get_netinterval().to_bits(), isolation_interval());
        ctest_host_set_phys_max_ticrate(0.0);
    }
}

// ---------------------------------------------------------------------------
// The server-message writers (host.c:522 / :542 / :569)

fn expect_string_message(bytes: &[u8], tag: u8, text: &str) {
    let mut want = vec![tag];
    want.extend_from_slice(text.as_bytes());
    want.push(0);
    assert_eq!(bytes, want.as_slice());
}

#[test]
fn sv_client_printf_writes_svc_print_to_the_current_client_only() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        ctest_host_reset_clients(4);
        ctest_host_set_host_client(2);
        let text = CString::new("hello world").unwrap();
        ctest_host_sv_client_printf(text.as_ptr());
    }
    expect_string_message(&client_msg(2), SVC_PRINT, "hello world");
    for i in [0, 1, 3] {
        assert!(client_msg(i).is_empty(), "client {i} must be untouched");
    }
}

#[test]
fn host_client_commands_writes_svc_stufftext() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        ctest_host_reset_clients(4);
        ctest_host_set_host_client(0);
        let text = CString::new("bf\n").unwrap();
        ctest_host_client_commands(text.as_ptr());
    }
    expect_string_message(&client_msg(0), SVC_STUFFTEXT, "bf\n");
}

#[test]
fn sv_broadcast_printf_skips_inactive_and_unspawned_clients() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        ctest_host_reset_clients(3);
        ctest_host_set_client_state(1, 0, 1); // inactive
        ctest_host_set_client_state(2, 1, 0); // active but not spawned
        let text = CString::new("everyone").unwrap();
        ctest_host_sv_broadcast_printf(text.as_ptr());
    }
    expect_string_message(&client_msg(0), SVC_PRINT, "everyone");
    assert!(client_msg(1).is_empty());
    assert!(client_msg(2).is_empty());
    // index 3 is inside the fixture array but outside svs.maxclients
    assert!(client_msg(3).is_empty());
}

#[test]
fn sv_broadcast_printf_honours_maxclients_not_the_array_length() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        ctest_host_reset_clients(2);
        let text = CString::new("two").unwrap();
        ctest_host_sv_broadcast_printf(text.as_ptr());
    }
    expect_string_message(&client_msg(0), SVC_PRINT, "two");
    expect_string_message(&client_msg(1), SVC_PRINT, "two");
    assert!(client_msg(2).is_empty());
    assert!(client_msg(3).is_empty());
}

// ---------------------------------------------------------------------------
// Host_Callback_Notify (host.c:417)

fn notify(name: &str, string: &str) {
    let n = CString::new(name).unwrap();
    let s = CString::new(string).unwrap();
    let mut var = CvarMirror {
        name: n.as_ptr(),
        string: s.as_ptr(),
        flags: 0,
        value: 0.0,
        default_string: core::ptr::null(),
        callback: None,
        next: core::ptr::null_mut(),
    };
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe { ctest_host_callback_notify(&mut var) };
}

#[test]
fn callback_notify_is_silent_without_an_active_server() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        ctest_host_reset_clients(2);
        ctest_host_set_sv_active(0);
    }
    notify("teamplay", "1");
    assert!(client_msg(0).is_empty());
    assert!(client_msg(1).is_empty());
}

#[test]
fn callback_notify_broadcasts_the_cvar_name_and_string() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        ctest_host_reset_clients(2);
        ctest_host_set_sv_active(1);
    }
    notify("teamplay", "1");
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe { ctest_host_set_sv_active(0) };
    expect_string_message(&client_msg(0), SVC_PRINT, "\"teamplay\" changed to \"1\"\n");
    expect_string_message(&client_msg(1), SVC_PRINT, "\"teamplay\" changed to \"1\"\n");
}

// ---------------------------------------------------------------------------
// Host_FindMaxClients (host.c:357)

#[test]
fn find_max_clients_defaults_to_a_single_disconnected_client() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        let exe = CString::new("vkquake").unwrap();
        ctest_host_register_deathmatch();
        ctest_host_set_cmdline(exe.as_ptr(), core::ptr::null(), core::ptr::null());
        ctest_host_find_max_clients();
        assert_eq!(ctest_host_get_maxclients(), 1);
        assert_eq!(ctest_host_get_cls_state(), CA_DISCONNECTED);
        assert_eq!(ctest_host_get_deathmatch().to_bits(), 0.0f32.to_bits());
        assert_eq!(deathmatch_string(), "0");
    }
}

#[test]
fn find_max_clients_reads_the_listen_argument_and_sets_deathmatch() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        let exe = CString::new("vkquake").unwrap();
        let listen = CString::new("-listen").unwrap();
        let n = CString::new("4").unwrap();
        ctest_host_register_deathmatch();
        ctest_host_set_cmdline(exe.as_ptr(), listen.as_ptr(), n.as_ptr());
        ctest_host_find_max_clients();
        assert_eq!(ctest_host_get_maxclients(), 4);
        assert_eq!(ctest_host_get_deathmatch().to_bits(), 1.0f32.to_bits());
        assert_eq!(deathmatch_string(), "1");

        // and back down again, so the "0" branch of host.c:397 is pinned too
        ctest_host_set_cmdline(exe.as_ptr(), core::ptr::null(), core::ptr::null());
        ctest_host_find_max_clients();
        assert_eq!(deathmatch_string(), "0");
    }
}

#[test]
fn find_max_clients_uses_eight_when_listen_is_the_last_argument() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        let exe = CString::new("vkquake").unwrap();
        let listen = CString::new("-listen").unwrap();
        ctest_host_set_cmdline(exe.as_ptr(), listen.as_ptr(), core::ptr::null());
        ctest_host_find_max_clients();
        assert_eq!(ctest_host_get_maxclients(), 8);
    }
}

#[test]
fn find_max_clients_clamps_to_the_scoreboard_size() {
    let _g = lock();
    // SAFETY: ADR-004. Drives the C oracle over its file-scope state, TEST_LOCK held.
    unsafe {
        let exe = CString::new("vkquake").unwrap();
        let listen = CString::new("-listen").unwrap();
        let n = CString::new("99").unwrap();
        ctest_host_set_cmdline(exe.as_ptr(), listen.as_ptr(), n.as_ptr());
        ctest_host_find_max_clients();
        assert_eq!(ctest_host_get_maxclients(), MAX_SCOREBOARD);

        // host.c:387 turns a non-positive count into 8, not into 1
        let zero = CString::new("0").unwrap();
        ctest_host_set_cmdline(exe.as_ptr(), listen.as_ptr(), zero.as_ptr());
        ctest_host_find_max_clients();
        assert_eq!(ctest_host_get_maxclients(), 8);
    }
}

// ---------------------------------------------------------------------------
// Host_WriteConfiguration (host.c:487)

fn config_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("vkQuake.cfg") // CONFIG_NAME, quakedef.h:33
}

fn write_config_into(dir: &std::path::Path, initialized: bool, errstate: c_int) -> Option<String> {
    let d = CString::new(dir.to_str().unwrap()).unwrap();
    let path = config_path(dir);
    let _ = std::fs::remove_file(&path);
    // SAFETY: ADR-004. Points host.c at a scratch gamedir and runs the writer; TEST_LOCK is held by the caller.
    unsafe {
        ctest_host_set_gamedir(d.as_ptr());
        ctest_host_set_parms(errstate);
        ctest_host_set_initialized(initialized as c_int);
        ctest_host_write_configuration();
        ctest_host_set_initialized(0);
    }
    std::fs::read_to_string(&path).ok()
}

#[test]
fn write_configuration_emits_the_trailing_state_commands() {
    let _g = lock();
    let dir = std::env::temp_dir().join("ctest_host_cfg_ok");
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: ADR-004. Reads the Key_WriteBindings call counter (stubs.c).
    let before = unsafe { ctest_host_key_write_bindings_calls() };
    let text = write_config_into(&dir, true, 0).expect("config must be written");
    // SAFETY: ADR-004. Reads the Key_WriteBindings call counter (stubs.c).
    let after = unsafe { ctest_host_key_write_bindings_calls() };
    // No cvar reachable from this binary carries CVAR_ARCHIVE and no binding
    // is set, so the file is exactly host.c:504-505's two commands in order.
    // host.c:494 opens the stream in text mode, so the newlines reach the
    // file as CRLF on Windows; the line sequence is the invariant.
    assert_eq!(text.replace("\r\n", "\n"), "vid_restart\n+mlook\n");
    assert_eq!(after - before, 1, "Key_WriteBindings must be called once");
    let _ = std::fs::remove_file(config_path(&dir));
}

#[test]
fn write_configuration_is_a_no_op_before_host_init_and_after_an_error() {
    let _g = lock();
    let dir = std::env::temp_dir().join("ctest_host_cfg_gate");
    std::fs::create_dir_all(&dir).unwrap();
    assert!(
        write_config_into(&dir, false, 0).is_none(),
        "host_initialized == false must write nothing"
    );
    assert!(
        write_config_into(&dir, true, 1).is_none(),
        "host_parms->errstate must suppress the write"
    );
}
