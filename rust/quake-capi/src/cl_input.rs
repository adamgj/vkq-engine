//! `Quake/cl_input.c` -- builds an intended movement command from the key
//! state (Rust migration Phase 7 M7, T7.2, Pattern A whole-file swap).
//!
//! ## ADR-009 raise-topology audit
//!
//! `cl_input.c` has zero direct raise sites (no `Host_Error`, `Host_EndGame`
//! or `Sys_Error`). Its entire ADR-009 surface is inside `CL_SendMove`:
//!
//! - the nine `MSG_Write*` families it calls all reach `SZ_GetSpace`
//!   (`net_msg.c:479`), which `Host_Error`s when the sizebuf disallows
//!   overflow. They are batched and replayed inside one guarded C frame
//!   (`ClInput_Glue_WriteBatch`).
//! - `CL_Disconnect` (`cl_input.c:558`) raises twice over -- its own
//!   `MSG_WriteByte` (`cl_main.c:391`) and, with a local server running,
//!   `Host_ShutdownServer` -> `SV_DropClient` -> `ClientDisconnect` QC. It
//!   goes through `ClInput_Glue_Disconnect`.
//!
//! So [`quake_rs_cl_send_move`] is the file's only status core, and
//! `Quake/cl_input_glue.c`'s `CL_SendMove` is the only `Host_Reraise` frame.
//! Everything else -- `Cmd_Argv`, `atoi`, `Con_Printf`, `Cmd_AddCommand2`,
//! `anglemod`, `V_StartPitchDrift`/`V_StopPitchDrift`,
//! `NET_QSocketGetProQuakeAngleHack` and `NET_SendUnreliableMessage` (the net
//! drivers report failure by return value and only ever `Sys_Error`, which
//! terminates rather than longjmping) -- is called straight from Rust and the
//! public entry points return `()` exactly as in C.
//!
//! ## Ownership
//!
//! ADR-007: `cl`/`cls` stay C-owned for all of T7.2; the row closes at T7.4.
//! The seventeen `kbutton_t` objects, `in_impulse` and the nine cvars keep
//! C-visible storage in `Quake/cl_input_glue.c`, because `in_sdl.c`, `menu.c`
//! and `view.c` still read them by name and `cl_main.c:1376-1390` still
//! registers the cvars (registration order is observable in `config.cfg`).
//! Nothing in this file owns storage.
//!
//! The mirror-typed externs (`cl`, `cls`, `UserCmd`) are declared here rather
//! than in `quake-c-sys`, which has no `[dependencies]` and so cannot name
//! `quake-types` -- the same finding `quake-c-sys/src/sv_user.rs` records.

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::cl_input as g;
use quake_c_sys::qboolean;
use quake_c_sys::sv_main::atoi;
use quake_c_sys::sv_send::NET_SendUnreliableMessage;
use quake_c_sys::sv_user::NET_QSocketGetProQuakeAngleHack;
use quake_math::mathlib as m;
use quake_types::host::{ClientState, ClientStatic, UserCmd};

/// A `Host_Guard` status: 0, or the code the guarded frame caught. Non-zero
/// must reach `Quake/cl_input_glue.c` untouched.
type Raise = c_int;

/// Propagate a non-zero `Host_Guard` status, abandoning the rest of the body
/// exactly where C's `longjmp` would have left it.
macro_rules! raise {
    ($e:expr) => {{
        let r: Raise = $e;
        if r != 0 {
            return r;
        }
    }};
}

extern "C" {
    /// ADR-007 rows closed in T7.4; storage in [`crate::cl_main`].
    static mut cl: ClientState;
    static mut cls: ClientStatic;
}

/// `client.h:68`.
const SIGNONS: c_int = 4;
/// `protocol.h` -- `clc_move`.
const CLC_MOVE: c_int = 3;
/// `protocol.h` -- `clcdp_ackframe`.
const CLCDP_ACKFRAME: c_int = 50;
/// `protocol.h:63`.
const PEXT2_PREDINFO: c_uint = 0x0000_0020;
/// `protocol.h` -- `PROTOCOL_NETQUAKE`.
const PROTOCOL_NETQUAKE: c_uint = 15;
/// `client.h:168` -- `countof (cl.movecmds) - 1`.
const MOVECMDS_MASK: c_int = 63;
const PITCH: usize = 0;
const YAW: usize = 1;
const ROLL: usize = 2;

#[inline]
unsafe fn cvar_value(var: *const c::cvar_t) -> c_float {
    // SAFETY: `var` is one of the glue-owned `cvar_t` objects; only `.value`
    // is read, and `cvar_t` outlives the process.
    unsafe { ptr::addr_of!((*var).value).read() }
}

// ---------------------------------------------------------------------------
// Key state machine (cl_input.c:61-118).
//
// state bit 0 is the current state of the key
// state bit 1 is edge triggered on the up to down transition
// state bit 2 is edge triggered on the down to up transition

/// `cl_input.c:61`. External linkage in C, so it stays exported.
///
/// # Safety
/// `b` must point at a live `kbutton_t`.
#[no_mangle]
pub unsafe extern "C" fn KeyDown(b: *mut g::kbutton_t) {
    // SAFETY: `b` is one of the glue-owned `kbutton_t` objects.
    unsafe {
        let cc = c::Cmd_Argv(1);
        let k: c_int = if *cc != 0 {
            atoi(cc)
        } else {
            -1 // typed manually at the console for continuous down
        };

        if k == (*b).down[0] || k == (*b).down[1] {
            return; // repeating key
        }

        if (*b).down[0] == 0 {
            (*b).down[0] = k;
        } else if (*b).down[1] == 0 {
            (*b).down[1] = k;
        } else {
            c::Con_Printf(c"Three keys down for a button!\n".as_ptr());
            return;
        }

        if (*b).state & 1 != 0 {
            return; // still down
        }
        (*b).state |= 1 + 2; // down + impulse down
    }
}

/// `cl_input.c:90`.
///
/// # Safety
/// `b` must point at a live `kbutton_t`.
#[no_mangle]
pub unsafe extern "C" fn KeyUp(b: *mut g::kbutton_t) {
    // SAFETY: `b` is one of the glue-owned `kbutton_t` objects.
    unsafe {
        let cc = c::Cmd_Argv(1);
        let k: c_int = if *cc != 0 {
            atoi(cc)
        } else {
            // typed manually at the console, assume for unsticking, so clear all
            (*b).down[0] = 0;
            (*b).down[1] = 0;
            (*b).state = 4; // impulse up
            return;
        };

        if (*b).down[0] == k {
            (*b).down[0] = 0;
        } else if (*b).down[1] == k {
            (*b).down[1] = 0;
        } else {
            return; // key up without coresponding down (menu pass through)
        }
        if (*b).down[0] != 0 || (*b).down[1] != 0 {
            return; // some other key is still holding it down
        }

        if (*b).state & 1 == 0 {
            return; // still up (this should not happen)
        }
        (*b).state &= !1; // now up
        (*b).state |= 4; // impulse up
    }
}

macro_rules! key_binding {
    ($down:ident, $up:ident, $button:ident) => {
        #[no_mangle]
        pub extern "C" fn $down() {
            // SAFETY: the glue-owned `kbutton_t` is a process-lifetime object.
            unsafe { KeyDown(ptr::addr_of_mut!(g::$button)) }
        }
        #[no_mangle]
        pub extern "C" fn $up() {
            // SAFETY: as above.
            unsafe { KeyUp(ptr::addr_of_mut!(g::$button)) }
        }
    };
}

key_binding!(IN_KLookDown, IN_KLookUp, in_klook);
key_binding!(IN_UpDown, IN_UpUp, in_up);
key_binding!(IN_DownDown, IN_DownUp, in_down);
key_binding!(IN_LeftDown, IN_LeftUp, in_left);
key_binding!(IN_RightDown, IN_RightUp, in_right);
key_binding!(IN_ForwardDown, IN_ForwardUp, in_forward);
key_binding!(IN_BackDown, IN_BackUp, in_back);
key_binding!(IN_LookupDown, IN_LookupUp, in_lookup);
key_binding!(IN_LookdownDown, IN_LookdownUp, in_lookdown);
key_binding!(IN_MoveleftDown, IN_MoveleftUp, in_moveleft);
key_binding!(IN_MoverightDown, IN_MoverightUp, in_moveright);
key_binding!(IN_SpeedDown, IN_SpeedUp, in_speed);
key_binding!(IN_StrafeDown, IN_StrafeUp, in_strafe);
key_binding!(IN_AttackDown, IN_AttackUp, in_attack);
key_binding!(IN_UseDown, IN_UseUp, in_use);
key_binding!(IN_JumpDown, IN_JumpUp, in_jump);

/// `cl_input.c:128`.
#[no_mangle]
pub extern "C" fn IN_MLookDown() {
    // SAFETY: glue-owned process-lifetime object.
    unsafe { KeyDown(ptr::addr_of_mut!(g::in_mlook)) }
}

/// `cl_input.c:132` -- the one handler with a body of its own.
#[no_mangle]
pub extern "C" fn IN_MLookUp() {
    // SAFETY: glue-owned process-lifetime objects; `V_StartPitchDrift` only
    // writes `cl`'s drift fields and cannot raise.
    unsafe {
        KeyUp(ptr::addr_of_mut!(g::in_mlook));
        if ptr::addr_of!(g::in_mlook.state).read() & 1 == 0
            && cvar_value(ptr::addr_of!(g::lookspring)) != 0.0
        {
            g::V_StartPitchDrift();
        }
    }
}

/// `cl_input.c:262`.
#[no_mangle]
pub extern "C" fn IN_Impulse() {
    // SAFETY: `Cmd_Argv` always returns a NUL-terminated string.
    unsafe { ptr::addr_of_mut!(g::in_impulse).write(atoi(c::Cmd_Argv(1))) }
}

/// `cl_input.c:277`. Returns 0.25 if a key was pressed and released during
/// the frame, 0.5 if it was pressed and held, 0 if held then released, and
/// 1.0 if held for the entire time.
///
/// # Safety
/// `key` must point at a live `kbutton_t`.
#[no_mangle]
pub unsafe extern "C" fn CL_KeyState(key: *mut g::kbutton_t) -> c_float {
    // SAFETY: `key` is one of the glue-owned `kbutton_t` objects.
    unsafe {
        let mut val: c_float;
        let impulsedown = (*key).state & 2 != 0;
        let impulseup = (*key).state & 4 != 0;
        let down = (*key).state & 1 != 0;
        val = 0.0;

        if impulsedown && !impulseup {
            if down {
                val = 0.5; // pressed and held this frame
            } else {
                val = 0.0; //	I_Error ();
            }
        }
        if impulseup && !impulsedown {
            if down {
                val = 0.0; //	I_Error ();
            } else {
                val = 0.0; // released this frame
            }
        }
        if !impulsedown && !impulseup {
            if down {
                val = 1.0; // held the entire frame
            } else {
                val = 0.0; // up the entire frame
            }
        }
        if impulsedown && impulseup {
            if down {
                val = 0.75; // released and re-pressed this frame
            } else {
                val = 0.25; // pressed and released this frame
            }
        }

        (*key).state &= 1; // clear impulses

        val
    }
}

// ---------------------------------------------------------------------------
// Angle and move construction.

/// `cl_input.c:344` -- true if the server sent a fixangle recently.
#[no_mangle]
pub extern "C" fn CL_AngleLocked() -> qboolean {
    // SAFETY: `cl` is C-owned storage with process lifetime (ADR-007).
    unsafe { cl.fixangle_time == cl.mtime[0] || cl.fixangle_time == cl.mtime[1] }
}

/// `cl_input.c:356` -- moves the local angle positions.
#[no_mangle]
pub extern "C" fn CL_AdjustAngles() {
    // SAFETY: every object touched here is glue-owned or C-owned with
    // process lifetime; `V_StopPitchDrift` cannot raise.
    unsafe {
        if CL_AngleLocked() {
            return;
        }

        let speed: c_float = if (ptr::addr_of!(g::in_speed.state).read() & 1)
            ^ ((cvar_value(ptr::addr_of!(g::cl_alwaysrun)) != 0.0) as c_int)
            != 0
        {
            // `host_frametime` is a double and the cvar a float, so C
            // promotes and the product is computed at double width before
            // the store to `float speed`.
            (c::host_frametime * cvar_value(ptr::addr_of!(g::cl_anglespeedkey)) as f64) as c_float
        } else {
            c::host_frametime as c_float
        };

        if ptr::addr_of!(g::in_strafe.state).read() & 1 == 0 {
            let yawspeed = cvar_value(ptr::addr_of!(g::cl_yawspeed));
            cl.viewangles[YAW] -= speed * yawspeed * CL_KeyState(ptr::addr_of_mut!(g::in_right));
            cl.viewangles[YAW] += speed * yawspeed * CL_KeyState(ptr::addr_of_mut!(g::in_left));
            cl.viewangles[YAW] = m::anglemod(cl.viewangles[YAW]);
        }
        if ptr::addr_of!(g::in_klook.state).read() & 1 != 0 {
            g::V_StopPitchDrift();
            let pitchspeed = cvar_value(ptr::addr_of!(g::cl_pitchspeed));
            cl.viewangles[PITCH] -=
                speed * pitchspeed * CL_KeyState(ptr::addr_of_mut!(g::in_forward));
            cl.viewangles[PITCH] += speed * pitchspeed * CL_KeyState(ptr::addr_of_mut!(g::in_back));
        }

        let up = CL_KeyState(ptr::addr_of_mut!(g::in_lookup));
        let down = CL_KeyState(ptr::addr_of_mut!(g::in_lookdown));

        let pitchspeed = cvar_value(ptr::addr_of!(g::cl_pitchspeed));
        cl.viewangles[PITCH] -= speed * pitchspeed * up;
        cl.viewangles[PITCH] += speed * pitchspeed * down;

        if up != 0.0 || down != 0.0 {
            g::V_StopPitchDrift();
        }

        // johnfitz -- variable pitch clamping
        let maxpitch = cvar_value(ptr::addr_of!(g::cl_maxpitch));
        if cl.viewangles[PITCH] > maxpitch {
            cl.viewangles[PITCH] = maxpitch;
        }
        let minpitch = cvar_value(ptr::addr_of!(g::cl_minpitch));
        if cl.viewangles[PITCH] < minpitch {
            cl.viewangles[PITCH] = minpitch;
        }
        // johnfitz

        // `f32::clamp` would be the same on finite input, but the two ifs are
        // what C spells and only they leave a NaN roll untouched in both
        // directions.
        #[allow(clippy::manual_clamp)]
        {
            if cl.viewangles[ROLL] > 50.0 {
                cl.viewangles[ROLL] = 50.0;
            }
            if cl.viewangles[ROLL] < -50.0 {
                cl.viewangles[ROLL] = -50.0;
            }
        }
    }
}

/// `cl_input.c:411` -- builds the intended movement message for the server.
///
/// # Safety
/// `cmd` must point at a writable `usercmd_t`.
#[no_mangle]
pub unsafe extern "C" fn CL_BaseMove(cmd: *mut UserCmd) {
    // SAFETY: `cmd` is caller-owned and writable; the button/cvar objects are
    // glue-owned with process lifetime.
    unsafe {
        ptr::write_bytes(cmd, 0, 1);

        (*cmd).viewangles = cl.viewangles;

        if cls.signon != SIGNONS {
            return;
        }

        let sidespeed = cvar_value(ptr::addr_of!(g::cl_sidespeed));
        if ptr::addr_of!(g::in_strafe.state).read() & 1 != 0 {
            (*cmd).sidemove += sidespeed * CL_KeyState(ptr::addr_of_mut!(g::in_right));
            (*cmd).sidemove -= sidespeed * CL_KeyState(ptr::addr_of_mut!(g::in_left));
        }

        (*cmd).sidemove += sidespeed * CL_KeyState(ptr::addr_of_mut!(g::in_moveright));
        (*cmd).sidemove -= sidespeed * CL_KeyState(ptr::addr_of_mut!(g::in_moveleft));

        let upspeed = cvar_value(ptr::addr_of!(g::cl_upspeed));
        (*cmd).upmove += upspeed * CL_KeyState(ptr::addr_of_mut!(g::in_up));
        (*cmd).upmove -= upspeed * CL_KeyState(ptr::addr_of_mut!(g::in_down));

        if ptr::addr_of!(g::in_klook.state).read() & 1 == 0 {
            (*cmd).forwardmove += cvar_value(ptr::addr_of!(g::cl_forwardspeed))
                * CL_KeyState(ptr::addr_of_mut!(g::in_forward));
            (*cmd).forwardmove -= cvar_value(ptr::addr_of!(g::cl_backspeed))
                * CL_KeyState(ptr::addr_of_mut!(g::in_back));
        }

        //
        // adjust for speed key
        //
        if (ptr::addr_of!(g::in_speed.state).read() & 1)
            ^ ((cvar_value(ptr::addr_of!(g::cl_alwaysrun)) != 0.0) as c_int)
            != 0
        {
            let movespeedkey = cvar_value(ptr::addr_of!(g::cl_movespeedkey));
            (*cmd).forwardmove *= movespeedkey;
            (*cmd).sidemove *= movespeedkey;
            (*cmd).upmove *= movespeedkey;
        }
    }
}

/// `cl_input.c:449`.
///
/// # Safety
/// `cmd` must point at a writable `usercmd_t`.
#[no_mangle]
pub unsafe extern "C" fn CL_FinishMove(cmd: *mut UserCmd) {
    // SAFETY: `cmd` is caller-owned and writable; the button objects are
    // glue-owned with process lifetime.
    unsafe {
        //
        // send button bits
        //
        let mut bits: c_uint = 0;

        if g::in_attack.state & 3 != 0 {
            bits |= 1;
        }
        g::in_attack.state &= !2;

        if g::in_jump.state & 3 != 0 {
            bits |= 2;
        }
        g::in_jump.state &= !2;

        if g::in_use.state & 3 != 0 {
            bits |= 4;
        }
        g::in_use.state &= !2;

        (*cmd).buttons = bits;
        (*cmd).impulse = ptr::addr_of!(g::in_impulse).read() as c_uint;

        ptr::addr_of_mut!(g::in_impulse).write(0);
    }
}

// ---------------------------------------------------------------------------
// CL_SendMove (cl_input.c:480) -- the file's only ADR-009 status core.

const WRITE_BATCH: usize = 64;
const W_BYTE: c_int = 0;
const W_SHORT: c_int = 1;
const W_LONG: c_int = 2;
const W_FLOAT: c_int = 3;
const W_ANGLE: c_int = 4;
const W_ANGLE16: c_int = 5;

/// Buffers `MSG_Write*` calls so they can be replayed inside one guarded C
/// frame (ADR-009): every one of them reaches `SZ_GetSpace`, which
/// `Host_Error`s on overflow, so none may run on a Rust frame. Ordering is
/// preserved exactly -- a full batch is flushed before the next op is
/// buffered, and a raise inside a flush leaves the same partial writes
/// committed that C's `longjmp` would have.
struct Writer {
    sb: *mut c::sizebuf_t,
    ops: [g::ClInputWriteOp; WRITE_BATCH],
    n: usize,
}

impl Writer {
    fn new(sb: *mut c::sizebuf_t) -> Self {
        Writer {
            sb,
            ops: [g::ClInputWriteOp {
                kind: 0,
                i: 0,
                f: 0.0,
                u: 0,
            }; WRITE_BATCH],
            n: 0,
        }
    }

    fn flush(&mut self) -> Raise {
        if self.n == 0 {
            return 0;
        }
        let count = self.n as c_int;
        self.n = 0;
        // SAFETY: `sb` outlives the writer and `ops[..count]` is initialised.
        unsafe { g::ClInput_Glue_WriteBatch(self.sb, self.ops.as_ptr(), count) }
    }

    fn push(&mut self, kind: c_int, i: c_int, f: c_float, u: c_uint) -> Raise {
        if self.n == WRITE_BATCH {
            raise!(self.flush());
        }
        self.ops[self.n] = g::ClInputWriteOp { kind, i, f, u };
        self.n += 1;
        0
    }

    fn byte(&mut self, v: c_int) -> Raise {
        self.push(W_BYTE, v, 0.0, 0)
    }
    fn short(&mut self, v: c_int) -> Raise {
        self.push(W_SHORT, v, 0.0, 0)
    }
    fn long(&mut self, v: c_int) -> Raise {
        self.push(W_LONG, v, 0.0, 0)
    }
    fn float(&mut self, v: c_float) -> Raise {
        self.push(W_FLOAT, 0, v, 0)
    }
    fn angle(&mut self, v: c_float, flags: c_uint) -> Raise {
        self.push(W_ANGLE, 0, v, flags)
    }
    fn angle16(&mut self, v: c_float, flags: c_uint) -> Raise {
        self.push(W_ANGLE16, 0, v, flags)
    }
}

/// `cl_input.c:480`. The non-reraising core behind
/// `Quake/cl_input_glue.c`'s `CL_SendMove`; Rust never calls the plain name.
///
/// # Safety
/// `cmd` is either null or points at a readable `usercmd_t`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_send_move(cmd: *const UserCmd) -> Raise {
    // SAFETY: `cl`/`cls` are C-owned with process lifetime; `buf`/`data` are
    // locals whose addresses never escape past the guarded write batch and
    // the `NET_SendUnreliableMessage` call.
    unsafe {
        let mut data = [0u8; 1024];
        let mut buf = c::sizebuf_t {
            // COMPAT: cl_input.c:483-488 leaves `allowoverflow`/`overflowed`
            // uninitialised. Rust cannot read an uninitialised local, so both
            // are zeroed. Only reachable with a >1024-byte ackframe run,
            // which `cl.ackframes_count` never produces.
            allowoverflow: false,
            overflowed: false,
            data: data.as_mut_ptr(),
            maxsize: data.len() as c_int,
            cursize: 0,
        };
        let mut w = Writer::new(ptr::addr_of_mut!(buf));

        let mut i: c_uint = 0;
        while i < cl.ackframes_count {
            raise!(w.byte(CLCDP_ACKFRAME));
            // COMPAT: `cl.ackframes` is `int[8]` but the loop bound is
            // `cl.ackframes_count`; cl_input.c:493 indexes it unchecked, so
            // the port reproduces the raw pointer arithmetic rather than
            // introducing a bounds check the oracle does not have.
            let ackframes = ptr::addr_of!(cl.ackframes) as *const c_int;
            raise!(w.long(ackframes.add(i as usize).read()));
            i += 1;
        }
        cl.ackframes_count = 0;

        if !cmd.is_null() {
            raise!(w.flush());
            let dump = buf.cursize;
            let bits: c_uint = (*cmd).buttons;

            //
            // send the movement message
            //
            raise!(w.byte(CLC_MOVE));

            if cl.protocol_pext2 & PEXT2_PREDINFO != 0 {
                // server will ack this once it has been applied to the player's entity state
                raise!(w.short(cl.movemessages & 0xffff));
                // so server can get cmd timing (pings will be calculated by entframe acks).
                raise!(w.float((*cmd).servertime));
            } else {
                raise!(w.float(cl.mtime[0] as c_float)); // so server can get ping times
            }

            for j in 0..3 {
                // johnfitz -- 16-bit angles for PROTOCOL_FITZQUAKE
                // spike -- nq+bjp3 use 8bit angles. all other supported protocols use 16bit ones.
                // spike -- proquake servers bump client->server angles up to at least 16bit. this is safe because it only happens when both client+server advertise
                // it, and because it never actually gets recorded into demos anyway. spike -- predinfo also always means 16bit angles, even if for some reason the
                // server doesn't advertise proquake (like dp).
                if cl.protocol == PROTOCOL_NETQUAKE
                    && !NET_QSocketGetProQuakeAngleHack(cls.netcon as *const c_void)
                    && cl.protocol_pext2 & PEXT2_PREDINFO == 0
                {
                    raise!(w.angle(cl.viewangles[j], cl.protocolflags));
                } else {
                    raise!(w.angle16(cl.viewangles[j], cl.protocolflags));
                }
            }
            // johnfitz

            raise!(w.short((*cmd).forwardmove as c_int));
            raise!(w.short((*cmd).sidemove as c_int));
            raise!(w.short((*cmd).upmove as c_int));

            raise!(w.byte((bits & 0xff) as c_int));
            raise!(w.byte(((*cmd).impulse & 0xff) as c_int));
            if bits & (1u32 << 30) != 0 {
                raise!(w.long((*cmd).weapon));
            }
            ptr::addr_of_mut!(g::in_impulse).write(0);

            cl.movecmds[(cl.movemessages & MOVECMDS_MASK) as usize] = *cmd;

            //
            // allways dump the first two message, because it may contain leftover inputs
            // from the last level
            //
            raise!(w.flush());
            cl.movemessages += 1;
            if cl.movemessages <= 2 {
                buf.cursize = dump;
            }
        }

        // fixme: nops if we're still connecting, or something.

        raise!(w.flush());

        //
        // deliver the message
        //
        if cls.demoplayback || buf.cursize == 0 {
            return 0;
        }

        if NET_SendUnreliableMessage(cls.netcon as *mut c_void, ptr::addr_of_mut!(buf).cast()) == -1
        {
            c::Con_Printf(c"CL_SendMove: lost server connection\n".as_ptr());
            raise!(g::ClInput_Glue_Disconnect());
        }

        0
    }
}

// ---------------------------------------------------------------------------
// Registration (cl_input.c:567).

/// `cmd.h:110` -- `Cmd_AddCommand (name, func)` is
/// `Cmd_AddCommand2 (name, func, src_command, false)`.
fn add_command(name: *const c_char, func: extern "C" fn()) {
    // SAFETY: `name` is a static NUL-terminated literal and `func` has the
    // `xcommand_t` signature.
    unsafe {
        c::Cmd_AddCommand2(
            name,
            Some(func as unsafe extern "C" fn()),
            c::cmd_source_t_src_command,
            false,
        );
    }
}

/// `cl_input.c:567`. The registration order is preserved verbatim; the cvars
/// are not registered here (`cl_main.c:1376-1390` owns that, and cvar
/// registration order is observable in `config.cfg`).
#[no_mangle]
pub extern "C" fn CL_InitInput() {
    add_command(c"+moveup".as_ptr(), IN_UpDown);
    add_command(c"-moveup".as_ptr(), IN_UpUp);
    add_command(c"+movedown".as_ptr(), IN_DownDown);
    add_command(c"-movedown".as_ptr(), IN_DownUp);
    add_command(c"+left".as_ptr(), IN_LeftDown);
    add_command(c"-left".as_ptr(), IN_LeftUp);
    add_command(c"+right".as_ptr(), IN_RightDown);
    add_command(c"-right".as_ptr(), IN_RightUp);
    add_command(c"+forward".as_ptr(), IN_ForwardDown);
    add_command(c"-forward".as_ptr(), IN_ForwardUp);
    add_command(c"+back".as_ptr(), IN_BackDown);
    add_command(c"-back".as_ptr(), IN_BackUp);
    add_command(c"+lookup".as_ptr(), IN_LookupDown);
    add_command(c"-lookup".as_ptr(), IN_LookupUp);
    add_command(c"+lookdown".as_ptr(), IN_LookdownDown);
    add_command(c"-lookdown".as_ptr(), IN_LookdownUp);
    add_command(c"+strafe".as_ptr(), IN_StrafeDown);
    add_command(c"-strafe".as_ptr(), IN_StrafeUp);
    add_command(c"+moveleft".as_ptr(), IN_MoveleftDown);
    add_command(c"-moveleft".as_ptr(), IN_MoveleftUp);
    add_command(c"+moveright".as_ptr(), IN_MoverightDown);
    add_command(c"-moveright".as_ptr(), IN_MoverightUp);
    add_command(c"+speed".as_ptr(), IN_SpeedDown);
    add_command(c"-speed".as_ptr(), IN_SpeedUp);
    add_command(c"+attack".as_ptr(), IN_AttackDown);
    add_command(c"-attack".as_ptr(), IN_AttackUp);
    add_command(c"+use".as_ptr(), IN_UseDown);
    add_command(c"-use".as_ptr(), IN_UseUp);
    add_command(c"+jump".as_ptr(), IN_JumpDown);
    add_command(c"-jump".as_ptr(), IN_JumpUp);
    add_command(c"impulse".as_ptr(), IN_Impulse);
    add_command(c"+klook".as_ptr(), IN_KLookDown);
    add_command(c"-klook".as_ptr(), IN_KLookUp);
    add_command(c"+mlook".as_ptr(), IN_MLookDown);
    add_command(c"-mlook".as_ptr(), IN_MLookUp);
}
