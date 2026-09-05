//! Differential/characterization gate for `Quake/cl_input.c` -- the client
//! input layer. Rust migration Phase 7, M7, task T7.2.
//!
//! The oracle fixture lives in `stubs/cl_input_ref.c`; read its module doc
//! first. The short version: `cl_input_glue.c` is `#ifdef USE_RUST_HOST` and
//! `cl_main.c` is an oracle source, so the fixture owns the plain twins of the
//! nine `cl_input` cvars, `cl_maxpitch`/`cl_minpitch`/`lookspring`, the
//! seventeen `kbutton_t` objects and `in_impulse`, and every seeder writes
//! both storages in a single call. That is what makes the one-pass driver
//! below correct: the oracle writes `c_ref_cl`/`c_ref_in_*`, the port writes
//! `cl`/`in_*`, and neither can see the other's state.
//!
//! ## Seeding, and the degenerate gate
//!
//! `Cvar_RegisterVariable` never runs in this link, so every `cvar_t.value` is
//! 0.0f from static init unless something seeds it -- and a differential where
//! both sides multiply by a zero speed passes while proving nothing.
//! `ctest_clinput_reset` therefore republishes the real engine defaults
//! (200/200/200/350/2.0/140/150/1.5/1.0/90/-90/0/500) into both storages, and
//! `cls.state` is set to `ca_connected` explicitly because `cactive_t` starts
//! at `ca_dedicated`. On top of that every test here asserts something
//! *positive* -- an exact expected float, a specific button transition, a
//! non-empty datagram, an exact byte count -- alongside the cross-side
//! comparison, so a test cannot pass on two identically-degenerate sides.
//!
//! ## ADR-010
//!
//! `cl_input.c` has no transcendentals; its only float-order-sensitive code is
//! `CL_AdjustAngles`' `speed * cvar * CL_KeyState (...)` chains (three-factor
//! products that must not be reassociated) and `anglemod`. Every angle and
//! `usercmd_t` assertion below compares `f32::to_bits`, never an epsilon.
//!
//! ## Raise topology (ADR-009)
//!
//! Nothing reachable from these entry points can raise in this link:
//! `Con_Printf` is inert, `Cmd_AddCommand2` has no `Sys_Error` path and
//! `host_initialized` is false, `NET_SendUnreliableMessage` always returns 1,
//! and `CL_SendMove`'s 1024-byte stack buffer is never close to `SZ_GetSpace`
//! overflow with at most 8 ackframes. The drivers therefore call the entry
//! points directly rather than through `Host_Guard`. The one exception is
//! `CL_SendMove`: `cl_input_ref.c`'s plain `CL_SendMove` is the reraising
//! `Host_Reraise (quake_rs_cl_send_move (cmd))` wrapper, and Rust must never
//! call a reraising wrapper, so this suite calls the status core
//! `quake_rs_cl_send_move` and asserts it returned 0.
//!
//! ## Known asymmetry, reported not papered over
//!
//! `NET_SendUnreliableMessage` in `stubs.c` can never return -1, so
//! `cl_input.c:557`'s "lost server connection" arm and its `CL_Disconnect`
//! call are unreachable on both sides. `ctest_clinput_disconnect_calls` is
//! asserted to stay at 0 so the suite at least fails if a port were to call
//! `CL_Disconnect` spuriously; proving the arm itself needs a settable return
//! in `stubs.c`, which this task may not edit.
//!
//! `CL_InitInput`'s *registration order* is likewise not observable:
//! `Cmd_AddCommand2` does a sorted insert, so the table cannot distinguish the
//! order the 35 calls were made in. Presence of all 35 names in both tables is
//! what `init_input_registers_every_command` checks.

use core::ffi::{c_char, c_double, c_float, c_int, c_uint, c_void};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// `stubs/cl_input_ref.c`: side 1 == oracle (`c_ref_*`), side 0 == the plain
/// storage the Rust port reads and writes.
const ORACLE: c_int = 1;
const RUST: c_int = 0;

const NBUTTONS: usize = 17;

/// Index order of `ctest_clinput_buttons[]`. Must match `cl_input_ref.c:283`.
const B_MLOOK: usize = 0;
const B_KLOOK: usize = 1;
const B_LEFT: usize = 2;
const B_RIGHT: usize = 3;
const B_FORWARD: usize = 4;
const B_BACK: usize = 5;
const B_LOOKUP: usize = 6;
const B_LOOKDOWN: usize = 7;
const B_MOVELEFT: usize = 8;
const B_MOVERIGHT: usize = 9;
const B_STRAFE: usize = 10;
const B_SPEED: usize = 11;
const B_USE: usize = 12;
const B_JUMP: usize = 13;
const B_ATTACK: usize = 14;
const B_UP: usize = 15;
const B_DOWN: usize = 16;

/// `ctest_clinput_set_cvars` argument order.
const CV_UPSPEED: usize = 0;
const CV_FORWARDSPEED: usize = 1;
const CV_BACKSPEED: usize = 2;
const CV_SIDESPEED: usize = 3;
const CV_MOVESPEEDKEY: usize = 4;
const CV_YAWSPEED: usize = 5;
const CV_PITCHSPEED: usize = 6;
const CV_ANGLESPEEDKEY: usize = 7;
const CV_ALWAYSRUN: usize = 8;
const CV_MAXPITCH: usize = 9;
const CV_MINPITCH: usize = 10;
const CV_LOOKSPRING: usize = 11;

const CVAR_DEFAULTS: [c_float; 13] = [
    200.0, 200.0, 200.0, 350.0, 2.0, 140.0, 150.0, 1.5, 1.0, 90.0, -90.0, 0.0, 500.0,
];

// The indices above are a contract with `ctest_clinput_set_cvars`, which
// writes both sides from one array; a test that only overrides some of them
// still depends on the rest landing in the right slots.
const _: () = assert!(
    CV_UPSPEED == 0
        && CV_FORWARDSPEED == 1
        && CV_BACKSPEED == 2
        && CV_SIDESPEED == 3
        && CV_MOVESPEEDKEY == 4
        && CV_YAWSPEED == 5
        && CV_PITCHSPEED == 6
        && CV_ANGLESPEEDKEY == 7
        && CV_ALWAYSRUN == 8
        && CV_MAXPITCH == 9
        && CV_MINPITCH == 10
        && CV_LOOKSPRING == 11
        && CVAR_DEFAULTS.len() == 13
);

/// `client.h:68` / `client.h:167`.
const SIGNONS: c_int = 4;
const MOVECMDS_MASK: c_int = 63;

/// `protocol.h`.
const PROTOCOL_NETQUAKE: c_uint = 15;
const PROTOCOL_FITZQUAKE: c_uint = 666;
const PROTOCOL_RMQ: c_uint = 999;
const PEXT2_PREDINFO: c_uint = 0x0000_0020;
const CLC_MOVE: u8 = 3;
const CLCDP_ACKFRAME: u8 = 50;

/// `usercmd_t` (protocol.h:416) as 32-bit slot indices, checked against
/// `ctest_clinput_usercmd_size` before use.
const U_SERVERTIME: usize = 0;
const U_VIEWANGLES: usize = 2;
const U_FORWARDMOVE: usize = 5;
const U_SIDEMOVE: usize = 6;
const U_UPMOVE: usize = 7;
const U_BUTTONS: usize = 11;
const U_IMPULSE: usize = 12;
const U_WEAPON: usize = 14;
const USERCMD_SIZE: usize = 60;

extern "C" {
    fn ctest_clinput_reset();
    fn ctest_clinput_button_addr(side: c_int, idx: c_int) -> *mut c_void;
    fn ctest_clinput_set_button(idx: c_int, down0: c_int, down1: c_int, state: c_int);
    fn ctest_clinput_get_button(side: c_int, idx: c_int, out: *mut c_int);
    fn ctest_clinput_set_impulse(v: c_int);
    fn ctest_clinput_get_impulse(side: c_int) -> c_int;
    fn ctest_clinput_set_cvars(v: *const c_float);
    fn ctest_clinput_set_angles(a: *const c_float);
    fn ctest_clinput_get_angles(side: c_int, out: *mut c_float);
    fn ctest_clinput_set_times(
        time: c_double,
        mtime0: c_double,
        mtime1: c_double,
        fixangle_time: c_double,
        frametime: c_double,
    );
    fn ctest_clinput_get_drift(side: c_int, out: *mut c_double);
    fn ctest_clinput_set_proto(
        protocol: c_uint,
        pext2: c_uint,
        protoflags: c_uint,
        signon: c_int,
        demoplayback: c_int,
        has_netcon: c_int,
    );
    fn ctest_clinput_set_ackframes(frames: *const c_int, count: c_int);
    fn ctest_clinput_get_ackframes_count(side: c_int) -> c_int;
    fn ctest_clinput_set_movemessages(n: c_int);
    fn ctest_clinput_get_movemessages(side: c_int) -> c_int;
    fn ctest_clinput_usercmd_size() -> c_int;
    #[allow(clippy::too_many_arguments)]
    fn ctest_clinput_make_cmd(
        out: *mut c_void,
        servertime: c_float,
        seconds: c_float,
        viewangles: *const c_float,
        forwardmove: c_float,
        sidemove: c_float,
        upmove: c_float,
        buttons: c_uint,
        impulse: c_uint,
        sequence: c_uint,
        weapon: c_int,
    );
    fn ctest_clinput_get_movecmd(side: c_int, idx: c_int, out: *mut c_void);
    fn ctest_clinput_tokenize(text: *const c_char);
    fn ctest_clinput_cmd_exists(side: c_int, name: *const c_char) -> c_int;
    fn ctest_clinput_disconnect_calls() -> c_int;

    // stubs.c datagram recorder -- shared by both sides, so CL_SendMove tests
    // reset it between the two runs.
    fn ctest_net_send_reset();
    fn ctest_net_send_calls() -> c_int;
    fn ctest_net_send_bytes(len: *mut c_int) -> *const u8;
    fn ctest_net_send_call_len(i: c_int) -> c_int;
    fn ctest_net_send_call_reliable(i: c_int) -> c_int;
    fn ctest_net_send_truncated() -> c_int;

    // The ported entry points (plain names are quake-capi #[no_mangle]
    // exports) and their oracle twins.
    fn KeyDown(b: *mut c_void);
    fn c_ref_KeyDown(b: *mut c_void);
    fn KeyUp(b: *mut c_void);
    fn c_ref_KeyUp(b: *mut c_void);
    fn CL_KeyState(b: *mut c_void) -> c_float;
    fn c_ref_CL_KeyState(b: *mut c_void) -> c_float;
    fn CL_AngleLocked() -> bool;
    fn c_ref_CL_AngleLocked() -> bool;
    fn CL_AdjustAngles();
    fn c_ref_CL_AdjustAngles();
    fn CL_BaseMove(cmd: *mut c_void);
    fn c_ref_CL_BaseMove(cmd: *mut c_void);
    fn CL_FinishMove(cmd: *mut c_void);
    fn c_ref_CL_FinishMove(cmd: *mut c_void);
    /// The status core, never `cl_input_ref.c`'s reraising `CL_SendMove`.
    fn quake_rs_cl_send_move(cmd: *const c_void) -> c_int;
    fn c_ref_CL_SendMove(cmd: *const c_void);
    fn CL_InitInput();
    fn c_ref_CL_InitInput();
}

extern "C" {
    // CL_InitInput's 35 handlers, in registration order.
    fn IN_UpDown();
    fn c_ref_IN_UpDown();
    fn IN_UpUp();
    fn c_ref_IN_UpUp();
    fn IN_DownDown();
    fn c_ref_IN_DownDown();
    fn IN_DownUp();
    fn c_ref_IN_DownUp();
    fn IN_LeftDown();
    fn c_ref_IN_LeftDown();
    fn IN_LeftUp();
    fn c_ref_IN_LeftUp();
    fn IN_RightDown();
    fn c_ref_IN_RightDown();
    fn IN_RightUp();
    fn c_ref_IN_RightUp();
    fn IN_ForwardDown();
    fn c_ref_IN_ForwardDown();
    fn IN_ForwardUp();
    fn c_ref_IN_ForwardUp();
    fn IN_BackDown();
    fn c_ref_IN_BackDown();
    fn IN_BackUp();
    fn c_ref_IN_BackUp();
    fn IN_LookupDown();
    fn c_ref_IN_LookupDown();
    fn IN_LookupUp();
    fn c_ref_IN_LookupUp();
    fn IN_LookdownDown();
    fn c_ref_IN_LookdownDown();
    fn IN_LookdownUp();
    fn c_ref_IN_LookdownUp();
    fn IN_StrafeDown();
    fn c_ref_IN_StrafeDown();
    fn IN_StrafeUp();
    fn c_ref_IN_StrafeUp();
    fn IN_MoveleftDown();
    fn c_ref_IN_MoveleftDown();
    fn IN_MoveleftUp();
    fn c_ref_IN_MoveleftUp();
    fn IN_MoverightDown();
    fn c_ref_IN_MoverightDown();
    fn IN_MoverightUp();
    fn c_ref_IN_MoverightUp();
    fn IN_SpeedDown();
    fn c_ref_IN_SpeedDown();
    fn IN_SpeedUp();
    fn c_ref_IN_SpeedUp();
    fn IN_AttackDown();
    fn c_ref_IN_AttackDown();
    fn IN_AttackUp();
    fn c_ref_IN_AttackUp();
    fn IN_UseDown();
    fn c_ref_IN_UseDown();
    fn IN_UseUp();
    fn c_ref_IN_UseUp();
    fn IN_JumpDown();
    fn c_ref_IN_JumpDown();
    fn IN_JumpUp();
    fn c_ref_IN_JumpUp();
    fn IN_Impulse();
    fn c_ref_IN_Impulse();
    fn IN_KLookDown();
    fn c_ref_IN_KLookDown();
    fn IN_KLookUp();
    fn c_ref_IN_KLookUp();
    fn IN_MLookDown();
    fn c_ref_IN_MLookDown();
    fn IN_MLookUp();
    fn c_ref_IN_MLookUp();
}

/// The command table `CL_InitInput` builds: name, port handler, oracle handler.
struct InCmd {
    name: &'static str,
    rust: unsafe extern "C" fn(),
    oracle: unsafe extern "C" fn(),
}

const IN_COMMANDS: [InCmd; 35] = [
    InCmd {
        name: "+moveup",
        rust: IN_UpDown,
        oracle: c_ref_IN_UpDown,
    },
    InCmd {
        name: "-moveup",
        rust: IN_UpUp,
        oracle: c_ref_IN_UpUp,
    },
    InCmd {
        name: "+movedown",
        rust: IN_DownDown,
        oracle: c_ref_IN_DownDown,
    },
    InCmd {
        name: "-movedown",
        rust: IN_DownUp,
        oracle: c_ref_IN_DownUp,
    },
    InCmd {
        name: "+left",
        rust: IN_LeftDown,
        oracle: c_ref_IN_LeftDown,
    },
    InCmd {
        name: "-left",
        rust: IN_LeftUp,
        oracle: c_ref_IN_LeftUp,
    },
    InCmd {
        name: "+right",
        rust: IN_RightDown,
        oracle: c_ref_IN_RightDown,
    },
    InCmd {
        name: "-right",
        rust: IN_RightUp,
        oracle: c_ref_IN_RightUp,
    },
    InCmd {
        name: "+forward",
        rust: IN_ForwardDown,
        oracle: c_ref_IN_ForwardDown,
    },
    InCmd {
        name: "-forward",
        rust: IN_ForwardUp,
        oracle: c_ref_IN_ForwardUp,
    },
    InCmd {
        name: "+back",
        rust: IN_BackDown,
        oracle: c_ref_IN_BackDown,
    },
    InCmd {
        name: "-back",
        rust: IN_BackUp,
        oracle: c_ref_IN_BackUp,
    },
    InCmd {
        name: "+lookup",
        rust: IN_LookupDown,
        oracle: c_ref_IN_LookupDown,
    },
    InCmd {
        name: "-lookup",
        rust: IN_LookupUp,
        oracle: c_ref_IN_LookupUp,
    },
    InCmd {
        name: "+lookdown",
        rust: IN_LookdownDown,
        oracle: c_ref_IN_LookdownDown,
    },
    InCmd {
        name: "-lookdown",
        rust: IN_LookdownUp,
        oracle: c_ref_IN_LookdownUp,
    },
    InCmd {
        name: "+strafe",
        rust: IN_StrafeDown,
        oracle: c_ref_IN_StrafeDown,
    },
    InCmd {
        name: "-strafe",
        rust: IN_StrafeUp,
        oracle: c_ref_IN_StrafeUp,
    },
    InCmd {
        name: "+moveleft",
        rust: IN_MoveleftDown,
        oracle: c_ref_IN_MoveleftDown,
    },
    InCmd {
        name: "-moveleft",
        rust: IN_MoveleftUp,
        oracle: c_ref_IN_MoveleftUp,
    },
    InCmd {
        name: "+moveright",
        rust: IN_MoverightDown,
        oracle: c_ref_IN_MoverightDown,
    },
    InCmd {
        name: "-moveright",
        rust: IN_MoverightUp,
        oracle: c_ref_IN_MoverightUp,
    },
    InCmd {
        name: "+speed",
        rust: IN_SpeedDown,
        oracle: c_ref_IN_SpeedDown,
    },
    InCmd {
        name: "-speed",
        rust: IN_SpeedUp,
        oracle: c_ref_IN_SpeedUp,
    },
    InCmd {
        name: "+attack",
        rust: IN_AttackDown,
        oracle: c_ref_IN_AttackDown,
    },
    InCmd {
        name: "-attack",
        rust: IN_AttackUp,
        oracle: c_ref_IN_AttackUp,
    },
    InCmd {
        name: "+use",
        rust: IN_UseDown,
        oracle: c_ref_IN_UseDown,
    },
    InCmd {
        name: "-use",
        rust: IN_UseUp,
        oracle: c_ref_IN_UseUp,
    },
    InCmd {
        name: "+jump",
        rust: IN_JumpDown,
        oracle: c_ref_IN_JumpDown,
    },
    InCmd {
        name: "-jump",
        rust: IN_JumpUp,
        oracle: c_ref_IN_JumpUp,
    },
    InCmd {
        name: "impulse",
        rust: IN_Impulse,
        oracle: c_ref_IN_Impulse,
    },
    InCmd {
        name: "+klook",
        rust: IN_KLookDown,
        oracle: c_ref_IN_KLookDown,
    },
    InCmd {
        name: "-klook",
        rust: IN_KLookUp,
        oracle: c_ref_IN_KLookUp,
    },
    InCmd {
        name: "+mlook",
        rust: IN_MLookDown,
        oracle: c_ref_IN_MLookDown,
    },
    InCmd {
        name: "-mlook",
        rust: IN_MLookUp,
        oracle: c_ref_IN_MLookUp,
    },
];

/// Down handler, up handler and the button each pair drives, as indices into
/// `IN_COMMANDS` / the `ctest_clinput_buttons` table.
const IN_PAIRS: [(usize, usize, usize); 16] = [
    (0, 1, B_UP),
    (2, 3, B_DOWN),
    (4, 5, B_LEFT),
    (6, 7, B_RIGHT),
    (8, 9, B_FORWARD),
    (10, 11, B_BACK),
    (12, 13, B_LOOKUP),
    (14, 15, B_LOOKDOWN),
    (16, 17, B_STRAFE),
    (18, 19, B_MOVELEFT),
    (20, 21, B_MOVERIGHT),
    (22, 23, B_SPEED),
    (24, 25, B_ATTACK),
    (26, 27, B_USE),
    (28, 29, B_JUMP),
    (31, 32, B_KLOOK),
];

fn reset() {
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_reset() }
}

/// The engine cvar defaults with a few entries replaced. Both storages are
/// written, so a test can never seed one side only.
fn seed_cvars(overrides: &[(usize, c_float)]) {
    let mut v = CVAR_DEFAULTS;
    for &(i, x) in overrides {
        v[i] = x;
    }
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_cvars(v.as_ptr()) }
}

fn tokenize(text: &str) {
    let c = std::ffi::CString::new(text).unwrap();
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_tokenize(c.as_ptr()) }
}

fn addr(side: c_int, idx: usize) -> *mut c_void {
    // SAFETY: a read-back accessor over the fixture's static storage.
    let p = unsafe { ctest_clinput_button_addr(side, idx as c_int) };
    assert!(!p.is_null(), "button {idx} has no storage on side {side}");
    p
}

fn button(side: c_int, idx: usize) -> [c_int; 3] {
    let mut out = [0 as c_int; 3];
    // SAFETY: a read-back accessor over the fixture's static storage.
    unsafe { ctest_clinput_get_button(side, idx as c_int, out.as_mut_ptr()) }
    out
}

fn buttons(side: c_int) -> [[c_int; 3]; NBUTTONS] {
    let mut all = [[0 as c_int; 3]; NBUTTONS];
    for (i, slot) in all.iter_mut().enumerate() {
        *slot = button(side, i);
    }
    all
}

fn angle_bits(side: c_int) -> [u32; 3] {
    let mut out = [0.0 as c_float; 3];
    // SAFETY: a read-back accessor over the fixture's static storage.
    unsafe { ctest_clinput_get_angles(side, out.as_mut_ptr()) }
    [out[0].to_bits(), out[1].to_bits(), out[2].to_bits()]
}

/// `{pitchvel, nodrift, driftmove, laststop}` as raw bits.
fn drift_bits(side: c_int) -> [u64; 4] {
    let mut out = [0.0 as c_double; 4];
    // SAFETY: a read-back accessor over the fixture's static storage.
    unsafe { ctest_clinput_get_drift(side, out.as_mut_ptr()) }
    [
        out[0].to_bits(),
        out[1].to_bits(),
        out[2].to_bits(),
        out[3].to_bits(),
    ]
}

fn set_angles(a: [c_float; 3]) {
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_angles(a.as_ptr()) }
}

fn key_down(idx: usize) {
    // SAFETY: calls the port and its oracle twin; nothing reachable
    // here can raise (see the module doc).
    unsafe {
        c_ref_KeyDown(addr(ORACLE, idx));
        KeyDown(addr(RUST, idx));
    }
}

fn key_up(idx: usize) {
    // SAFETY: calls the port and its oracle twin; nothing reachable
    // here can raise (see the module doc).
    unsafe {
        c_ref_KeyUp(addr(ORACLE, idx));
        KeyUp(addr(RUST, idx));
    }
}

/// Both sides agree, *and* the shared expectation holds. The second half is
/// what stops a pair of identically-degenerate implementations from passing.
fn both(idx: usize, expect: [c_int; 3], what: &str) {
    assert_eq!(
        button(ORACLE, idx),
        button(RUST, idx),
        "{what}: sides differ"
    );
    assert_eq!(button(RUST, idx), expect, "{what}: unexpected value");
}

fn cmd_buf() -> Vec<u8> {
    assert_eq!(
        // SAFETY: a read-back accessor over the fixture's static storage.
        unsafe { ctest_clinput_usercmd_size() } as usize,
        USERCMD_SIZE,
        "usercmd_t layout changed; the U_* slot indices are stale"
    );
    vec![0u8; USERCMD_SIZE]
}

fn slot_u32(buf: &[u8], slot: usize) -> u32 {
    u32::from_le_bytes(buf[slot * 4..slot * 4 + 4].try_into().unwrap())
}

fn slot_f32(buf: &[u8], slot: usize) -> f32 {
    f32::from_bits(slot_u32(buf, slot))
}

#[allow(clippy::too_many_arguments)]
fn make_cmd(
    servertime: c_float,
    viewangles: [c_float; 3],
    forwardmove: c_float,
    sidemove: c_float,
    upmove: c_float,
    buttons: c_uint,
    impulse: c_uint,
    weapon: c_int,
) -> Vec<u8> {
    let mut buf = cmd_buf();
    // SAFETY: seeds both sides' static fixture storage.
    unsafe {
        ctest_clinput_make_cmd(
            buf.as_mut_ptr().cast(),
            servertime,
            0.05,
            viewangles.as_ptr(),
            forwardmove,
            sidemove,
            upmove,
            buttons,
            impulse,
            7,
            weapon,
        )
    }
    buf
}

fn movecmd(side: c_int, idx: c_int) -> Vec<u8> {
    let mut buf = cmd_buf();
    // SAFETY: a read-back accessor over the fixture's static storage.
    unsafe { ctest_clinput_get_movecmd(side, idx, buf.as_mut_ptr().cast()) }
    buf
}

/// What `stubs.c`'s datagram recorder saw for one side's `CL_SendMove`.
#[derive(Clone, PartialEq, Eq, Debug)]
struct NetObs {
    calls: c_int,
    lens: Vec<c_int>,
    reliable: Vec<c_int>,
    bytes: Vec<u8>,
    truncated: c_int,
}

fn net_obs() -> NetObs {
    // SAFETY: a read-back accessor over the fixture's static storage.
    unsafe {
        let calls = ctest_net_send_calls();
        let mut len = 0 as c_int;
        let p = ctest_net_send_bytes(&mut len);
        let bytes = if len > 0 {
            std::slice::from_raw_parts(p, len as usize).to_vec()
        } else {
            Vec::new()
        };
        NetObs {
            calls,
            lens: (0..calls).map(|i| ctest_net_send_call_len(i)).collect(),
            reliable: (0..calls)
                .map(|i| ctest_net_send_call_reliable(i))
                .collect(),
            bytes,
            truncated: ctest_net_send_truncated(),
        }
    }
}

/// Runs both `CL_SendMove` implementations against the same seeded state. The
/// recorder is process-global, so the two runs are separated by a reset --
/// unlike every other driver here, which can run both sides in one pass.
fn send_both(cmd: Option<&[u8]>) -> (NetObs, NetObs) {
    let p = match cmd {
        Some(c) => c.as_ptr().cast::<c_void>(),
        None => std::ptr::null(),
    };
    // SAFETY: calls the port and its oracle twin; nothing reachable
    // here can raise (see the module doc).
    unsafe {
        ctest_net_send_reset();
        c_ref_CL_SendMove(p);
        let oracle = net_obs();

        ctest_net_send_reset();
        let status = quake_rs_cl_send_move(p);
        assert_eq!(status, 0, "quake_rs_cl_send_move raised");
        let rust = net_obs();

        assert_eq!(
            ctest_clinput_disconnect_calls(),
            0,
            "CL_Disconnect is unreachable while NET_SendUnreliableMessage cannot fail"
        );
        (oracle, rust)
    }
}

/// `q_minmax.h:69`.
fn q_rint(x: f64) -> i32 {
    if x > 0.0 {
        (x + 0.5) as i32
    } else {
        (x - 0.5) as i32
    }
}

/// `net_msg.c:205` with `flags == 0`.
fn angle16(f: c_float) -> [u8; 2] {
    let v = (q_rint(f as f64 * 65536.0 / 360.0) & 65535) as u16;
    v.to_le_bytes()
}

/// `net_msg.c:194` with `flags == 0`.
fn angle8(f: c_float) -> u8 {
    (q_rint(f as f64 * 256.0 / 360.0) & 255) as u8
}

fn short_le(v: i32) -> [u8; 2] {
    [(v & 0xff) as u8, ((v >> 8) & 0xff) as u8]
}

fn long_le(v: i32) -> [u8; 4] {
    v.to_le_bytes()
}

/* --------------------------------------------------------------------------
 * 1. CL_KeyState -- the whole 3-bit state space.
 */

#[test]
fn key_state_covers_every_impulse_combination() {
    let _g = lock();

    // cl_input.c:277. Index is (impulseup<<2)|(impulsedown<<1)|down.
    const EXPECT: [c_float; 8] = [0.0, 1.0, 0.0, 0.5, 0.0, 0.0, 0.25, 0.75];

    for state in 0..8 as c_int {
        reset();
        // Non-zero down[] so the "impulses cleared, down[] untouched"
        // postcondition is actually visible.
        // SAFETY: seeds both sides' static fixture storage.
        unsafe { ctest_clinput_set_button(B_ATTACK as c_int, 3, 9, state) };

        // SAFETY: calls the port and its oracle twin; nothing reachable
        // here can raise (see the module doc).
        let o = unsafe { c_ref_CL_KeyState(addr(ORACLE, B_ATTACK)) };
        // SAFETY: calls the port and its oracle twin; nothing reachable
        // here can raise (see the module doc).
        let r = unsafe { CL_KeyState(addr(RUST, B_ATTACK)) };

        assert_eq!(o.to_bits(), r.to_bits(), "state {state}: sides differ");
        assert_eq!(
            r.to_bits(),
            EXPECT[state as usize].to_bits(),
            "state {state}: wrong fraction"
        );
        both(B_ATTACK, [3, 9, state & 1], &format!("state {state} after"));
    }
}

/* --------------------------------------------------------------------------
 * 2. KeyDown / KeyUp -- argv handling, repeats, the two-key slots and the
 *    console unstick path.
 */

#[test]
fn key_down_up_two_slot_sequence() {
    let _g = lock();
    reset();

    tokenize("+forward 7");
    key_down(B_FORWARD);
    both(B_FORWARD, [7, 0, 3], "first press");

    // Repeating key: no slot taken, no impulse re-armed.
    key_down(B_FORWARD);
    both(B_FORWARD, [7, 0, 3], "repeat of the same key");

    tokenize("+forward 9");
    key_down(B_FORWARD);
    both(B_FORWARD, [7, 9, 3], "second key takes down[1]");

    // Three keys down: refused, state untouched.
    tokenize("+forward 5");
    key_down(B_FORWARD);
    both(B_FORWARD, [7, 9, 3], "third key refused");

    // Releasing one of two held keys leaves the button down.
    tokenize("-forward 7");
    key_up(B_FORWARD);
    both(B_FORWARD, [0, 9, 3], "one of two released");

    // Releasing the last one clears bit 0 and arms the up impulse.
    tokenize("-forward 9");
    key_up(B_FORWARD);
    both(B_FORWARD, [0, 0, 6], "last key released");

    // Key-up without a matching down: menu pass-through, nothing changes.
    tokenize("-forward 4");
    key_up(B_FORWARD);
    both(B_FORWARD, [0, 0, 6], "unmatched release");
}

#[test]
fn key_down_up_empty_argv_console_paths() {
    let _g = lock();
    reset();

    // Empty Cmd_Argv(1) means "typed at the console": KeyDown uses k == -1.
    tokenize("+back");
    key_down(B_BACK);
    both(B_BACK, [-1, 0, 3], "console down");

    tokenize("+back");
    key_down(B_BACK);
    both(B_BACK, [-1, 0, 3], "console down repeats");

    // KeyUp with no argument unsticks: both slots cleared, state forced to 4
    // even though bit 0 was set.
    tokenize("-back");
    key_up(B_BACK);
    both(B_BACK, [0, 0, 4], "console unstick");

    // ... and it forces state to 4 unconditionally, from a fully idle button.
    reset();
    tokenize("-back");
    key_up(B_BACK);
    both(B_BACK, [0, 0, 4], "console unstick from idle");
}

/* --------------------------------------------------------------------------
 * 3. The IN_* handlers -- which kbutton_t each one actually touches.
 */

#[test]
fn in_handlers_press_and_release_their_own_button() {
    let _g = lock();

    for &(down, up, idx) in IN_PAIRS.iter() {
        reset();
        let before = buttons(RUST);

        tokenize(&format!("{} 11", IN_COMMANDS[down].name));
        // SAFETY: fixture access over static storage, serialised by TEST_LOCK.
        unsafe {
            (IN_COMMANDS[down].oracle)();
            (IN_COMMANDS[down].rust)();
        }
        both(idx, [11, 0, 3], IN_COMMANDS[down].name);
        assert_eq!(
            buttons(ORACLE),
            buttons(RUST),
            "{}: some other button diverged",
            IN_COMMANDS[down].name
        );
        let mut expect = before;
        expect[idx] = [11, 0, 3];
        assert_eq!(
            buttons(RUST),
            expect,
            "{}: touched a button it should not have",
            IN_COMMANDS[down].name
        );

        tokenize(&format!("{} 11", IN_COMMANDS[up].name));
        // SAFETY: fixture access over static storage, serialised by TEST_LOCK.
        unsafe {
            (IN_COMMANDS[up].oracle)();
            (IN_COMMANDS[up].rust)();
        }
        // The press armed bit 1; the release clears bit 0 and adds bit 2
        // without clearing bit 1, so the resting state is 6, not 4.
        both(idx, [0, 0, 6], IN_COMMANDS[up].name);
        expect[idx] = [0, 0, 6];
        assert_eq!(
            buttons(RUST),
            expect,
            "{}: touched a button it should not have",
            IN_COMMANDS[up].name
        );
    }
}

#[test]
fn mlook_handlers_start_pitch_drift_only_when_lookspring_is_set() {
    let _g = lock();

    // in_mlook starts with state == 1 from its static initializer, so the
    // press leaves bit 1 clear -- the release is what matters here.
    for &(lookspring, expect_pitchvel) in &[(0.0 as c_float, 0.0 as c_float), (1.0, 500.0)] {
        reset();
        seed_cvars(&[(CV_LOOKSPRING, lookspring)]);
        // V_StartPitchDrift no-ops while cl.laststop == cl.time, and reset
        // leaves both at 0.
        // SAFETY: seeds both sides' static fixture storage.
        unsafe { ctest_clinput_set_times(1.0, 0.0, 0.0, -1.0, 0.0) };

        tokenize("+mlook 3");
        // SAFETY: calls the port and its oracle twin; nothing reachable
        // here can raise (see the module doc).
        unsafe {
            c_ref_IN_MLookDown();
            IN_MLookDown();
        }
        both(B_MLOOK, [3, 0, 1], "mlook press");

        tokenize("-mlook 3");
        // SAFETY: calls the port and its oracle twin; nothing reachable
        // here can raise (see the module doc).
        unsafe {
            c_ref_IN_MLookUp();
            IN_MLookUp();
        }
        both(B_MLOOK, [0, 0, 4], "mlook release");

        assert_eq!(
            drift_bits(ORACLE),
            drift_bits(RUST),
            "lookspring {lookspring}: drift state differs"
        );
        assert_eq!(
            f64::from_bits(drift_bits(RUST)[0]),
            expect_pitchvel as f64,
            "lookspring {lookspring}: wrong pitchvel"
        );
    }
}

#[test]
fn impulse_handler_parses_argv_and_is_consumed_by_finish_move() {
    let _g = lock();

    for &(text, expect) in &[
        ("impulse 42", 42 as c_int),
        ("impulse -3", -3),
        ("impulse notanumber", 0),
        ("impulse", 0),
    ] {
        reset();
        // SAFETY: seeds both sides' static fixture storage.
        unsafe { ctest_clinput_set_impulse(99) };
        tokenize(text);
        // SAFETY: calls the port and its oracle twin; nothing reachable
        // here can raise (see the module doc).
        unsafe {
            c_ref_IN_Impulse();
            IN_Impulse();
        }
        // SAFETY: a read-back accessor over the fixture's static storage.
        let o = unsafe { ctest_clinput_get_impulse(ORACLE) };
        // SAFETY: a read-back accessor over the fixture's static storage.
        let r = unsafe { ctest_clinput_get_impulse(RUST) };
        assert_eq!(o, r, "{text}: sides differ");
        assert_eq!(r, expect, "{text}: wrong impulse");
    }
}

/* --------------------------------------------------------------------------
 * 4. CL_AngleLocked / CL_AdjustAngles.
 */

/// `mathlib.c:147`, transcribed with its exact widths: the multiply and the
/// scale are `double`, the truncation is `(int)`, the store is `float`.
// COMPAT: ADR-010 -- operation order and intermediate widths are observable.
fn anglemod(a: c_float) -> c_float {
    ((360.0f64 / 65536.0) * ((((a as f64) * (65536.0f64 / 360.0)) as i32 & 65535) as f64)) as f32
}

#[test]
fn angle_locked_matches_either_server_frame() {
    let _g = lock();

    for &(fixangle, m0, m1, expect) in &[
        (-1.0f64, 0.0f64, 0.0f64, false),
        (2.0, 2.0, 3.0, true),
        (3.0, 2.0, 3.0, true),
        (4.0, 2.0, 3.0, false),
        (0.0, 0.0, 0.0, true),
    ] {
        reset();
        // SAFETY: seeds both sides' static fixture storage.
        unsafe { ctest_clinput_set_times(0.0, m0, m1, fixangle, 0.1) };
        // SAFETY: calls the port and its oracle twin; nothing reachable
        // here can raise (see the module doc).
        let o = unsafe { c_ref_CL_AngleLocked() };
        // SAFETY: calls the port and its oracle twin; nothing reachable
        // here can raise (see the module doc).
        let r = unsafe { CL_AngleLocked() };
        assert_eq!(o, r, "fixangle {fixangle}: sides differ");
        assert_eq!(r, expect, "fixangle {fixangle}: wrong verdict");
    }
}

#[test]
fn adjust_angles_returns_early_while_locked() {
    let _g = lock();
    reset();
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_times(3.0, 2.0, 5.0, 2.0, 0.1) };
    set_angles([11.0, 22.0, 33.0]);
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_RIGHT as c_int, 1, 0, 3) };

    // SAFETY: calls the port and its oracle twin; nothing reachable
    // here can raise (see the module doc).
    unsafe {
        c_ref_CL_AdjustAngles();
        CL_AdjustAngles();
    }

    assert_eq!(angle_bits(ORACLE), angle_bits(RUST));
    assert_eq!(
        angle_bits(RUST),
        [11.0f32.to_bits(), 22.0f32.to_bits(), 33.0f32.to_bits()],
        "locked angles must not move"
    );
    // The early return happens before any CL_KeyState call, so the impulse
    // bit survives. That is what distinguishes "returned early" from
    // "ran and happened to compute zero".
    both(B_RIGHT, [1, 0, 3], "locked");
}

#[test]
fn adjust_angles_yaw_honours_speed_key_xor_alwaysrun() {
    let _g = lock();

    let frametime = 0.1f64;
    let mut seen = Vec::new();

    for &(speed_down, alwaysrun, keyed) in &[
        (0 as c_int, 0.0 as c_float, false),
        (1, 0.0, true),
        (0, 1.0, true),
        (1, 1.0, false),
    ] {
        reset();
        seed_cvars(&[(CV_ALWAYSRUN, alwaysrun)]);
        // SAFETY: seeds both sides' static fixture storage.
        unsafe { ctest_clinput_set_times(0.0, 0.0, 0.0, -1.0, frametime) };
        // SAFETY: seeds both sides' static fixture storage.
        unsafe { ctest_clinput_set_button(B_SPEED as c_int, 0, 0, speed_down) };
        // SAFETY: seeds both sides' static fixture storage.
        unsafe { ctest_clinput_set_button(B_RIGHT as c_int, 1, 0, 1) };
        set_angles([0.0, 0.0, 0.0]);

        // SAFETY: calls the port and its oracle twin; nothing reachable
        // here can raise (see the module doc).
        unsafe {
            c_ref_CL_AdjustAngles();
            CL_AdjustAngles();
        }

        let speed = if keyed {
            (frametime * 1.5f64) as c_float
        } else {
            frametime as c_float
        };
        let expect = anglemod(0.0f32 - (speed * 140.0f32) * 1.0f32);

        assert_eq!(
            angle_bits(ORACLE),
            angle_bits(RUST),
            "speed_down {speed_down} alwaysrun {alwaysrun}: sides differ"
        );
        assert_eq!(
            angle_bits(RUST)[1],
            expect.to_bits(),
            "speed_down {speed_down} alwaysrun {alwaysrun}: wrong yaw"
        );
        seen.push(angle_bits(RUST)[1]);
    }

    // The keyed and unkeyed yaws must actually differ, or the XOR above would
    // be untested.
    assert_ne!(seen[0], seen[1], "speed key made no difference");
    assert_eq!(seen[0], seen[3]);
    assert_eq!(seen[1], seen[2]);
}

#[test]
fn adjust_angles_strafe_suppresses_yaw_and_its_key_reads() {
    let _g = lock();
    reset();
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_times(0.0, 0.0, 0.0, -1.0, 0.1) };
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_STRAFE as c_int, 1, 0, 1) };
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_RIGHT as c_int, 1, 0, 3) };
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_LEFT as c_int, 2, 0, 3) };
    set_angles([0.0, 77.0, 0.0]);

    // SAFETY: calls the port and its oracle twin; nothing reachable
    // here can raise (see the module doc).
    unsafe {
        c_ref_CL_AdjustAngles();
        CL_AdjustAngles();
    }

    assert_eq!(angle_bits(ORACLE), angle_bits(RUST));
    // Not even anglemod runs, so the yaw keeps its exact seeded bits.
    assert_eq!(angle_bits(RUST)[1], 77.0f32.to_bits());
    both(B_RIGHT, [1, 0, 3], "strafe held");
    both(B_LEFT, [2, 0, 3], "strafe held");
}

#[test]
fn adjust_angles_klook_and_lookkeys_drive_pitch_and_stop_drift() {
    let _g = lock();

    let frametime = 0.125f64;
    reset();
    // Wide clamps so the arithmetic, not the clamp, is what is compared.
    seed_cvars(&[(CV_MAXPITCH, 1000.0), (CV_MINPITCH, -1000.0)]);
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_times(3.0, 0.0, 0.0, -1.0, frametime) };
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_KLOOK as c_int, 1, 0, 1) };
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_FORWARD as c_int, 1, 0, 1) }; // 1.0
                                                                      // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_BACK as c_int, 2, 0, 3) }; // 0.5
                                                                   // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_LOOKUP as c_int, 3, 0, 7) }; // 0.75
                                                                     // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_LOOKDOWN as c_int, 4, 0, 6) }; // 0.25
    set_angles([0.0, 0.0, 0.0]);

    // SAFETY: calls the port and its oracle twin; nothing reachable
    // here can raise (see the module doc).
    unsafe {
        c_ref_CL_AdjustAngles();
        CL_AdjustAngles();
    }

    // in_speed is up but cl_alwaysrun defaults to 1, so the XOR is true and
    // the angle speed key applies.
    let speed = (frametime * 1.5f64) as c_float;
    let mut pitch = 0.0f32;
    pitch -= (speed * 150.0f32) * 1.0f32;
    pitch += (speed * 150.0f32) * 0.5f32;
    pitch -= (speed * 150.0f32) * 0.75f32;
    pitch += (speed * 150.0f32) * 0.25f32;

    assert_eq!(angle_bits(ORACLE), angle_bits(RUST));
    assert_eq!(angle_bits(RUST)[0], pitch.to_bits(), "wrong pitch");
    assert_ne!(pitch, 0.0, "the pitch chain cancelled itself out");

    // Both the klook arm and the up/down arm call V_StopPitchDrift, which
    // stamps cl.laststop with cl.time and sets nodrift.
    assert_eq!(drift_bits(ORACLE), drift_bits(RUST));
    assert_eq!(f64::from_bits(drift_bits(RUST)[3]), 3.0, "laststop");
    assert_eq!(f64::from_bits(drift_bits(RUST)[1]), 1.0, "nodrift");
}

#[test]
fn adjust_angles_clamps_pitch_and_roll() {
    let _g = lock();

    for &(seed_pitch, seed_roll, expect_pitch, expect_roll) in &[
        (
            0.0 as c_float,
            80.0 as c_float,
            -17.0 as c_float,
            50.0 as c_float,
        ),
        (0.0, -80.0, -17.0, -50.0),
        (0.0, 12.5, -17.0, 12.5),
    ] {
        reset();
        seed_cvars(&[(CV_MAXPITCH, 42.0), (CV_MINPITCH, -17.0)]);
        // SAFETY: seeds both sides' static fixture storage.
        unsafe { ctest_clinput_set_times(0.0, 0.0, 0.0, -1.0, 1.0) };
        // 1.0 * 150 * 1.0 == -150 degrees of pitch, well past cl_minpitch.
        // SAFETY: seeds both sides' static fixture storage.
        unsafe { ctest_clinput_set_button(B_LOOKUP as c_int, 1, 0, 1) };
        set_angles([seed_pitch, 0.0, seed_roll]);

        // SAFETY: calls the port and its oracle twin; nothing reachable
        // here can raise (see the module doc).
        unsafe {
            c_ref_CL_AdjustAngles();
            CL_AdjustAngles();
        }

        assert_eq!(angle_bits(ORACLE), angle_bits(RUST), "roll {seed_roll}");
        assert_eq!(
            angle_bits(RUST),
            [
                expect_pitch.to_bits(),
                0.0f32.to_bits(),
                expect_roll.to_bits()
            ],
            "roll {seed_roll}"
        );
    }

    // ... and the opposite pitch clamp.
    reset();
    seed_cvars(&[(CV_MAXPITCH, 42.0), (CV_MINPITCH, -17.0)]);
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_times(0.0, 0.0, 0.0, -1.0, 1.0) };
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_LOOKDOWN as c_int, 1, 0, 1) };
    set_angles([0.0, 0.0, 0.0]);
    // SAFETY: calls the port and its oracle twin; nothing reachable
    // here can raise (see the module doc).
    unsafe {
        c_ref_CL_AdjustAngles();
        CL_AdjustAngles();
    }
    assert_eq!(angle_bits(ORACLE), angle_bits(RUST));
    assert_eq!(angle_bits(RUST)[0], 42.0f32.to_bits(), "cl_maxpitch");
}

/* --------------------------------------------------------------------------
 * 5. CL_BaseMove / CL_FinishMove.
 */

/// Drives both implementations against the same seeded state and returns the
/// two whole `usercmd_t` images. The seeded accumulator fields in `make_cmd`
/// are what proves `CL_BaseMove`'s leading `memset` really ran.
fn base_move_both() -> (Vec<u8>, Vec<u8>) {
    let mut oracle = make_cmd(9.0, [1.0, 2.0, 3.0], 4.0, 5.0, 6.0, 7, 8, 9);
    let mut rust = make_cmd(9.0, [1.0, 2.0, 3.0], 4.0, 5.0, 6.0, 7, 8, 9);
    // SAFETY: calls the port and its oracle twin; nothing reachable
    // here can raise (see the module doc).
    unsafe {
        c_ref_CL_BaseMove(oracle.as_mut_ptr().cast());
        CL_BaseMove(rust.as_mut_ptr().cast());
    }
    (oracle, rust)
}

#[test]
fn base_move_before_signon_copies_angles_and_nothing_else() {
    let _g = lock();
    reset();
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_proto(PROTOCOL_FITZQUAKE, 0, 0, SIGNONS - 1, 0, 1) };
    set_angles([13.5, -7.25, 0.5]);
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_FORWARD as c_int, 1, 0, 3) };

    let (oracle, rust) = base_move_both();
    assert_eq!(oracle, rust, "sides differ");

    let mut expect = vec![0u8; USERCMD_SIZE];
    expect[U_VIEWANGLES * 4..U_VIEWANGLES * 4 + 4].copy_from_slice(&13.5f32.to_le_bytes());
    expect[(U_VIEWANGLES + 1) * 4..(U_VIEWANGLES + 1) * 4 + 4]
        .copy_from_slice(&(-7.25f32).to_le_bytes());
    expect[(U_VIEWANGLES + 2) * 4..(U_VIEWANGLES + 2) * 4 + 4]
        .copy_from_slice(&0.5f32.to_le_bytes());
    assert_eq!(rust, expect, "memset + VectorCopy only");

    // Returning before the first CL_KeyState leaves every impulse bit armed.
    both(B_FORWARD, [1, 0, 3], "pre-signon");
}

#[test]
fn base_move_sums_every_axis_and_applies_the_speed_key() {
    let _g = lock();
    reset();
    set_angles([0.25, 0.5, 0.75]);
    // in_speed up, cl_alwaysrun 1 -> the XOR is true, so the speed key scales.
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_FORWARD as c_int, 1, 0, 1) }; // 1.0
                                                                      // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_BACK as c_int, 2, 0, 3) }; // 0.5
                                                                   // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_MOVERIGHT as c_int, 3, 0, 7) }; // 0.75
                                                                        // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_MOVELEFT as c_int, 4, 0, 6) }; // 0.25
                                                                       // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_UP as c_int, 5, 0, 1) }; // 1.0

    let (oracle, rust) = base_move_both();
    assert_eq!(oracle, rust, "sides differ");

    // (200*1 - 200*0.5) * 2, (350*0.75 - 350*0.25) * 2, (200*1 - 0) * 2.
    assert_eq!(slot_f32(&rust, U_FORWARDMOVE).to_bits(), 200.0f32.to_bits());
    assert_eq!(slot_f32(&rust, U_SIDEMOVE).to_bits(), 350.0f32.to_bits());
    assert_eq!(slot_f32(&rust, U_UPMOVE).to_bits(), 400.0f32.to_bits());
    assert_eq!(slot_f32(&rust, U_VIEWANGLES).to_bits(), 0.25f32.to_bits());
    // buttons/impulse/weapon are the memset's job, not CL_BaseMove's.
    assert_eq!(slot_u32(&rust, U_BUTTONS), 0);
    assert_eq!(slot_u32(&rust, U_SERVERTIME), 0);

    // Every key CL_BaseMove read has had its impulse bits cleared.
    assert_eq!(buttons(ORACLE), buttons(RUST));
    both(B_BACK, [2, 0, 1], "after base move");
    both(B_MOVERIGHT, [3, 0, 1], "after base move");
    both(B_MOVELEFT, [4, 0, 0], "after base move");
}

#[test]
fn base_move_strafe_and_klook_reroute_their_axes() {
    let _g = lock();
    reset();
    seed_cvars(&[(CV_ALWAYSRUN, 0.0)]); // no speed key, so the sums are raw
                                        // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_STRAFE as c_int, 1, 0, 1) };
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_KLOOK as c_int, 2, 0, 1) };
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_RIGHT as c_int, 3, 0, 1) }; // 1.0
                                                                    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_LEFT as c_int, 4, 0, 3) }; // 0.5
                                                                   // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_FORWARD as c_int, 5, 0, 1) };

    let (oracle, rust) = base_move_both();
    assert_eq!(oracle, rust, "sides differ");

    // 350*1.0 - 350*0.5 routed to sidemove by in_strafe.
    assert_eq!(slot_f32(&rust, U_SIDEMOVE).to_bits(), 175.0f32.to_bits());
    // in_klook skips the forward/back arm entirely, so in_forward is held but
    // contributes nothing -- and keeps its state, never having been read.
    assert_eq!(slot_f32(&rust, U_FORWARDMOVE).to_bits(), 0.0f32.to_bits());
    both(B_FORWARD, [5, 0, 1], "klook held");
    // The yaw keys were read by the strafe arm, so their impulses are gone.
    both(B_LEFT, [4, 0, 1], "strafe held");
}

#[test]
fn finish_move_packs_button_bits_and_consumes_the_impulse() {
    let _g = lock();
    reset();
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_ATTACK as c_int, 1, 0, 2) }; // impulse only
                                                                     // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_JUMP as c_int, 2, 0, 1) }; // held
                                                                   // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_button(B_USE as c_int, 3, 0, 4) }; // released
                                                                  // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_impulse(33) };

    let mut oracle = make_cmd(9.5, [1.0, 2.0, 3.0], 4.0, 5.0, 6.0, 0xdead, 0xbeef, 21);
    let mut rust = make_cmd(9.5, [1.0, 2.0, 3.0], 4.0, 5.0, 6.0, 0xdead, 0xbeef, 21);
    // SAFETY: calls the port and its oracle twin; nothing reachable
    // here can raise (see the module doc).
    unsafe {
        c_ref_CL_FinishMove(oracle.as_mut_ptr().cast());
        CL_FinishMove(rust.as_mut_ptr().cast());
    }

    assert_eq!(oracle, rust, "sides differ");
    assert_eq!(slot_u32(&rust, U_BUTTONS), 1 | 2, "attack + jump, not use");
    assert_eq!(slot_u32(&rust, U_IMPULSE), 33);
    // CL_FinishMove does not memset, so everything else survives untouched.
    assert_eq!(slot_f32(&rust, U_SERVERTIME).to_bits(), 9.5f32.to_bits());
    assert_eq!(slot_u32(&rust, U_WEAPON), 21);

    // Bit 1 is cleared on all three buttons; bit 0 and bit 2 are not.
    assert_eq!(buttons(ORACLE), buttons(RUST));
    both(B_ATTACK, [1, 0, 0], "after finish move");
    both(B_JUMP, [2, 0, 1], "after finish move");
    both(B_USE, [3, 0, 4], "after finish move");

    // SAFETY: a read-back accessor over the fixture's static storage.
    let o = unsafe { ctest_clinput_get_impulse(ORACLE) };
    // SAFETY: a read-back accessor over the fixture's static storage.
    let r = unsafe { ctest_clinput_get_impulse(RUST) };
    assert_eq!(o, r);
    assert_eq!(r, 0, "in_impulse must be consumed");
}

/* --------------------------------------------------------------------------
 * 6. CL_SendMove -- the wire format.
 */

fn seed_send(
    protocol: c_uint,
    pext2: c_uint,
    demoplayback: c_int,
    movemessages: c_int,
    acks: &[c_int],
    angles: [c_float; 3],
    mtime0: c_double,
) {
    reset();
    // SAFETY: seeds both sides' static fixture storage.
    unsafe {
        ctest_clinput_set_proto(protocol, pext2, 0, SIGNONS, demoplayback, 1);
        ctest_clinput_set_times(0.0, mtime0, 0.0, -1.0, 0.0);
        ctest_clinput_set_movemessages(movemessages);
        ctest_clinput_set_ackframes(acks.as_ptr(), acks.len() as c_int);
        ctest_clinput_set_angles(angles.as_ptr());
    }
}

fn ackframe_bytes(acks: &[c_int]) -> Vec<u8> {
    let mut out = Vec::new();
    for &a in acks {
        out.push(CLCDP_ACKFRAME);
        out.extend_from_slice(&long_le(a));
    }
    out
}

fn assert_delivered(oracle: &NetObs, rust: &NetObs, expect: &[u8]) {
    assert_eq!(oracle, rust, "sides differ");
    assert_eq!(rust.truncated, 0, "recorder overflowed");
    assert_eq!(rust.calls, 1, "expected exactly one datagram");
    assert_eq!(rust.reliable, vec![0 as c_int], "must be unreliable");
    assert_eq!(rust.lens, vec![expect.len() as c_int]);
    assert_eq!(rust.bytes, expect, "wire bytes");
}

#[test]
fn send_move_fitzquake_layout() {
    let _g = lock();
    let acks = [111 as c_int, -2];
    let angles = [10.0 as c_float, 20.0, 30.0];
    seed_send(PROTOCOL_FITZQUAKE, 0, 0, 5, &acks, angles, 1.5);
    // SAFETY: seeds both sides' static fixture storage.
    unsafe { ctest_clinput_set_impulse(77) };

    let cmd = make_cmd(0.25, [1.0, 2.0, 3.0], 100.5, -50.25, 12.0, 5, 9, 0x4321);
    let (oracle, rust) = send_both(Some(&cmd));

    let mut expect = ackframe_bytes(&acks);
    expect.push(CLC_MOVE);
    expect.extend_from_slice(&(1.5f64 as f32).to_le_bytes()); // cl.mtime[0]
    for a in angles {
        expect.extend_from_slice(&angle16(a));
    }
    expect.extend_from_slice(&short_le(100)); // (int)100.5
    expect.extend_from_slice(&short_le(-50)); // (int)-50.25
    expect.extend_from_slice(&short_le(12));
    expect.push(5); // buttons & 0xff
    expect.push(9); // impulse & 0xff
    assert_delivered(&oracle, &rust, &expect);

    for side in [ORACLE, RUST] {
        assert_eq!(
            // SAFETY: a read-back accessor over the fixture's static storage.
            unsafe { ctest_clinput_get_ackframes_count(side) },
            0,
            "ackframes must be drained"
        );
        // SAFETY: a read-back accessor over the fixture's static storage.
        assert_eq!(unsafe { ctest_clinput_get_movemessages(side) }, 6);
        assert_eq!(
            movecmd(side, 5 & MOVECMDS_MASK),
            cmd,
            "the ringbuffer slot must hold the whole usercmd_t"
        );
        assert_eq!(
            // SAFETY: a read-back accessor over the fixture's static storage.
            unsafe { ctest_clinput_get_impulse(side) },
            0,
            "CL_SendMove clears in_impulse a second time"
        );
    }
}

#[test]
fn send_move_angle_width_follows_protocol_and_predinfo() {
    let _g = lock();
    let angles = [10.0 as c_float, -20.5, 359.75];
    let cmd = make_cmd(0.5, [0.0, 0.0, 0.0], 1.0, 2.0, 3.0, 0, 0, 0);

    let mut widths = Vec::new();
    for &(protocol, pext2, wide) in &[
        (PROTOCOL_NETQUAKE, 0 as c_uint, false),
        (PROTOCOL_FITZQUAKE, 0, true),
        (PROTOCOL_RMQ, 0, true),
        (PROTOCOL_NETQUAKE, PEXT2_PREDINFO, true),
    ] {
        seed_send(protocol, pext2, 0, 0x1234, &[], angles, 2.25);
        let (oracle, rust) = send_both(Some(&cmd));

        let mut expect = vec![CLC_MOVE];
        if pext2 & PEXT2_PREDINFO != 0 {
            expect.extend_from_slice(&short_le(0x1234));
            expect.extend_from_slice(&0.5f32.to_le_bytes()); // cmd->servertime
        } else {
            expect.extend_from_slice(&(2.25f64 as f32).to_le_bytes()); // cl.mtime[0]
        }
        for a in angles {
            if wide {
                expect.extend_from_slice(&angle16(a));
            } else {
                expect.push(angle8(a));
            }
        }
        expect.extend_from_slice(&short_le(1));
        expect.extend_from_slice(&short_le(2));
        expect.extend_from_slice(&short_le(3));
        expect.push(0);
        expect.push(0);
        assert_delivered(&oracle, &rust, &expect);
        widths.push(rust.bytes.len());
    }

    // 8-bit angles really are narrower, and PEXT2_PREDINFO really does widen
    // PROTOCOL_NETQUAKE; otherwise all four cases would be the same test.
    assert_eq!(widths[0] + 3, widths[1]);
    assert_eq!(widths[1], widths[2]);
    assert_eq!(widths[3], widths[1] + 2, "predinfo adds the sequence short");
}

#[test]
fn send_move_writes_the_weapon_long_on_bit_30() {
    let _g = lock();
    seed_send(PROTOCOL_FITZQUAKE, 0, 0, 9, &[], [0.0, 0.0, 0.0], 0.0);
    let cmd = make_cmd(0.0, [0.0; 3], 0.0, 0.0, 0.0, (1u32 << 30) | 7, 0, -5);
    let (oracle, rust) = send_both(Some(&cmd));

    let mut expect = vec![CLC_MOVE];
    expect.extend_from_slice(&0.0f32.to_le_bytes());
    for _ in 0..3 {
        expect.extend_from_slice(&angle16(0.0));
    }
    for _ in 0..3 {
        expect.extend_from_slice(&short_le(0));
    }
    expect.push(7); // bits & 0xff -- bit 30 is not in the byte
    expect.push(0);
    expect.extend_from_slice(&long_le(-5)); // cmd->weapon
    assert_delivered(&oracle, &rust, &expect);
    assert_eq!(rust.bytes.len(), 23);
}

#[test]
fn send_move_dumps_the_first_two_messages() {
    let _g = lock();
    let acks = [7 as c_int];
    let cmd = make_cmd(1.0, [0.0; 3], 11.0, 0.0, 0.0, 0, 0, 0);

    // movemessages 0 and 1 roll the buffer back to the ackframe prefix.
    for start in [0 as c_int, 1] {
        seed_send(PROTOCOL_FITZQUAKE, 0, 0, start, &acks, [0.0; 3], 0.0);
        let (oracle, rust) = send_both(Some(&cmd));
        assert_delivered(&oracle, &rust, &ackframe_bytes(&acks));
        for side in [ORACLE, RUST] {
            // SAFETY: a read-back accessor over the fixture's static storage.
            assert_eq!(unsafe { ctest_clinput_get_movemessages(side) }, start + 1);
            assert_eq!(
                movecmd(side, start & MOVECMDS_MASK),
                cmd,
                "the journal is written even when the datagram is dumped"
            );
        }
    }

    // movemessages 2 is the first one that survives.
    seed_send(PROTOCOL_FITZQUAKE, 0, 0, 2, &acks, [0.0; 3], 0.0);
    let (oracle, rust) = send_both(Some(&cmd));
    assert_eq!(oracle, rust);
    assert_eq!(rust.calls, 1);
    assert_eq!(
        rust.bytes.len(),
        ackframe_bytes(&acks).len() + 19,
        "the move payload survives from the third message on"
    );

    // With nothing to keep, the dumped buffer is empty and nothing is sent.
    seed_send(PROTOCOL_FITZQUAKE, 0, 0, 0, &[], [0.0; 3], 0.0);
    let (oracle, rust) = send_both(Some(&cmd));
    assert_eq!(oracle, rust);
    assert_eq!(rust.calls, 0, "an empty buffer is not delivered");
    assert_eq!(movecmd(RUST, 0), cmd);
}

#[test]
fn send_move_demoplayback_and_null_cmd() {
    let _g = lock();
    let acks = [3 as c_int, 4, 5];
    let cmd = make_cmd(1.0, [0.0; 3], 1.0, 0.0, 0.0, 0, 0, 0);

    // Demo playback still journals the command, it just never transmits.
    seed_send(PROTOCOL_FITZQUAKE, 0, 1, 40, &acks, [0.0; 3], 0.0);
    let (oracle, rust) = send_both(Some(&cmd));
    assert_eq!(oracle, rust);
    assert_eq!(rust.calls, 0, "demoplayback must not transmit");
    for side in [ORACLE, RUST] {
        // SAFETY: a read-back accessor over the fixture's static storage.
        assert_eq!(unsafe { ctest_clinput_get_movemessages(side) }, 41);
        // SAFETY: a read-back accessor over the fixture's static storage.
        assert_eq!(unsafe { ctest_clinput_get_ackframes_count(side) }, 0);
        assert_eq!(movecmd(side, 40 & MOVECMDS_MASK), cmd);
    }

    // A NULL cmd flushes the ackframes and touches nothing else.
    seed_send(PROTOCOL_FITZQUAKE, 0, 0, 40, &acks, [0.0; 3], 0.0);
    let (oracle, rust) = send_both(None);
    assert_delivered(&oracle, &rust, &ackframe_bytes(&acks));
    for side in [ORACLE, RUST] {
        assert_eq!(
            // SAFETY: a read-back accessor over the fixture's static storage.
            unsafe { ctest_clinput_get_movemessages(side) },
            40,
            "no cmd, no journal entry"
        );
        // SAFETY: a read-back accessor over the fixture's static storage.
        assert_eq!(unsafe { ctest_clinput_get_ackframes_count(side) }, 0);
    }

    // No cmd and no ackframes: nothing at all goes out.
    seed_send(PROTOCOL_FITZQUAKE, 0, 0, 40, &[], [0.0; 3], 0.0);
    let (oracle, rust) = send_both(None);
    assert_eq!(oracle, rust);
    assert_eq!(rust.calls, 0);
}

/* --------------------------------------------------------------------------
 * 7. CL_InitInput.
 */

#[test]
fn init_input_registers_every_command() {
    let _g = lock();
    reset();

    for c in IN_COMMANDS.iter() {
        let name = std::ffi::CString::new(c.name).unwrap();
        assert_eq!(
            // SAFETY: a read-back accessor over the fixture's static storage.
            unsafe { ctest_clinput_cmd_exists(ORACLE, name.as_ptr()) },
            // SAFETY: a read-back accessor over the fixture's static storage.
            unsafe { ctest_clinput_cmd_exists(RUST, name.as_ptr()) },
            "{}: tables disagree before CL_InitInput",
            c.name
        );
    }

    // SAFETY: calls the port and its oracle twin; nothing reachable
    // here can raise (see the module doc).
    unsafe {
        c_ref_CL_InitInput();
        CL_InitInput();
    }

    for c in IN_COMMANDS.iter() {
        let name = std::ffi::CString::new(c.name).unwrap();
        // SAFETY: a read-back accessor over the fixture's static storage.
        let o = unsafe { ctest_clinput_cmd_exists(ORACLE, name.as_ptr()) };
        // SAFETY: a read-back accessor over the fixture's static storage.
        let r = unsafe { ctest_clinput_cmd_exists(RUST, name.as_ptr()) };
        assert_eq!(o, r, "{}: tables disagree", c.name);
        assert_eq!(r, 1, "{} was not registered", c.name);
    }

    // A name cl_input.c never registers must be absent from both tables, or
    // the loop above would pass against a table that answers yes to anything.
    let absent = std::ffi::CString::new("+notacommand").unwrap();
    assert_eq!(
        // SAFETY: a read-back accessor over the fixture's static storage.
        unsafe { ctest_clinput_cmd_exists(ORACLE, absent.as_ptr()) },
        0
    );
    assert_eq!(
        // SAFETY: a read-back accessor over the fixture's static storage.
        unsafe { ctest_clinput_cmd_exists(RUST, absent.as_ptr()) },
        0
    );
}
