//! `Quake/cl_demo.c` -- demo recording and playback (Rust migration Phase 7
//! M7, T7.4).
//!
//! # ADR-009 (raise topology)
//!
//! `cl_demo.c` has no `Host_Error` / `Host_EndGame` of its own; its only
//! failure exit is `Sys_Error ("Demo message > MAX_MSGLEN")` at
//! `cl_demo.c:175`/`:196`, and `Sys_Error` terminates rather than longjmping,
//! so it is called straight from Rust (the `world.c` / `sv_phys.c` /
//! `sv_send.c` precedent). Everything raise-capable this module reaches is a
//! *callee*, so every core below returns the `Host_Guard` status verbatim and
//! `Quake/cl_demo_glue.c` re-issues the jump from a pure C frame.
//! `Host_Reraise` therefore does not appear here.
//!
//! # ADR-007 (dual views)
//!
//! T7.4 closes the `cl`/`cls` row: both objects are defined by
//! [`crate::cl_main`] and this module reaches them through `addr_of_mut!`, the
//! same way `sv_send.rs` reaches `sv`/`svs` after M6. Everything else
//! `cl_demo.c` touched (`cl_lightstyle`, `cl_dlights`, `cl_temp_entities`,
//! `cl_beams`, `net_message`) stays C storage and is reached through externs.
//!
//! # ADR-010 (determinism)
//!
//! This module calls no libm function -- `cl_demo.c` has none. The float work
//! is the seek-offset arithmetic (`cl_demo.c:236`, `:247`), the static-sound
//! attenuation `CLAMP` (`:446`) and the timedemo fps division (`:840`); each
//! is transliterated at its C width and in its C operation order.
//!
//! # ADR-005 (float formatter)
//!
//! The only float-bearing format string reachable from here is
//! `"%i frames %5.1f seconds %5.1f fps\n"` (`cl_demo.c:840`). `%f` is safe;
//! there is no `%g` or `%e` anywhere in `cl_demo.c`.
//!
//! # Function-local statics
//!
//! `cl_demo.c` had two file-scope objects with internal linkage: `name`
//! (`:37`), shared by `CL_Record_f`, `CL_Resume_Record` and `CL_PlayDemo_f`,
//! and `weirdaltbufferthatprobablyisntneeded` (`:584`), the alternate record
//! buffer. Both become Rust statics with the same lifetime and the same
//! single-threaded access pattern the C had.

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::cl_demo as g;
use quake_types::host::{ClientState, ClientStatic, EntityOpaque};
use quake_types::progs::EntityState;

use crate::cl_main::{cl, cls};
use crate::view::Entity;

/// A `Host_Guard` status, propagated to `cl_demo_glue.c` untouched (ADR-009).
type Raise = c_int;

/// Propagate a non-zero guard status.
macro_rules! raise {
    ($e:expr) => {{
        let r: Raise = $e;
        if r != 0 {
            return r;
        }
    }};
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// `client.h:68`.
const SIGNONS: c_int = 4;
/// `quakedef.h:72`.
#[cfg(not(feature = "net"))]
const MAX_MSGLEN: c_int = 64000;
/// `net.h:36`.
const NET_MAXMESSAGE: usize = 64000;
/// `client.h:104-106` -- `cactive_t`.
const CA_DISCONNECTED: c_int = 1;
const CA_CONNECTED: c_int = 2;
/// `keys.h:138` -- `key_game`.
const KEY_GAME: c_int = 0;
/// `cmd.h` -- `src_command`.
const SRC_COMMAND: c_uint = 1;
/// `bspfile.h:276`.
const NUM_AMBIENTS: c_int = 4;
/// `quakedef.h:92-95`, `:110`.
const MAX_LIGHTSTYLES: usize = 64;
const MAX_SOUNDS: c_int = 2048;
const MAX_PARTICLETYPES: usize = 2048;
const MAX_CL_STATS: usize = 256;
/// `client.h:70`, `:85`, `:330`.
const MAX_DLIGHTS: usize = 64;
const MAX_BEAMS: usize = 32;
const MAX_TEMP_ENTITIES: usize = 256;

/// `protocol.h:35-41`.
const PROTOCOL_NETQUAKE: c_uint = 15;
const PROTOCOL_FITZQUAKE: c_uint = 666;
const PROTOCOL_RMQ: c_uint = 999;
/// `('F' << 0) + ('T' << 8) + ('E' << 16) + ('2' << 24)`.
const PROTOCOL_FTE_PEXT2: c_int =
    (b'F' as c_int) + ((b'T' as c_int) << 8) + ((b'E' as c_int) << 16) + ((b'2' as c_int) << 24);
/// `protocol.h:60-61`.
const PEXT2_REPLACEMENTDELTAS: c_uint = 0x0000_0008;
const PEXT2_PREDINFO: c_uint = 0x0000_0020;

/// `protocol.h:246-336` -- the svc opcodes emitted by this module.
const SVC_NOP: u8 = 1;
const SVC_DISCONNECT: c_int = 2;
const SVC_UPDATESTAT: c_int = 3;
const SVC_SETVIEW: c_int = 5;
const SVC_STUFFTEXT: c_int = 9;
const SVC_SERVERINFO: c_int = 11;
const SVC_LIGHTSTYLE: c_int = 12;
const SVC_UPDATENAME: c_int = 13;
const SVC_UPDATEFRAGS: c_int = 14;
const SVC_UPDATECOLORS: c_int = 17;
const SVC_SIGNONNUM: c_int = 25;
const SVC_SPAWNSTATICSOUND: c_int = 29;
const SVC_SPAWNSTATICSOUND2: c_int = 44;
const SVCDP_UPDATESTATBYTE: c_int = 51;
const SVCDP_PRECACHE: c_int = 54;
const SVCFTE_UPDATESTATFLOAT: c_int = 79;

/// How many pending `MSG_Write*` ops are batched into one guarded C call.
///
/// Purely a Rust-side buffering choice: the ops replay in insertion order
/// inside `ClDemo_Glue_WriteBatch`, so the emitted byte stream is identical
/// for any batch size.
const WRITE_BATCH: usize = 64;

/// `SEEK_SET` / `SEEK_END`.
const SEEK_SET: c_int = 0;
const SEEK_END: c_int = 2;

// ---------------------------------------------------------------------------
// C storage this module reaches
// ---------------------------------------------------------------------------

extern "C" {
    /// `Quake/protocol.c` -- `entity_state_t nullentitystate;` (note: not all
    /// null).
    static nullentitystate: EntityState;
    /// `Quake/cl_main_glue.c` (`cl_main.c:59`).
    static mut cl_dlights: [c::cl_tent::dlight_t; MAX_DLIGHTS];
    /// `Quake/cl_tent_glue.c` (`cl_tent.c:27`).
    static mut cl_temp_entities: [EntityOpaque; MAX_TEMP_ENTITIES];
}

// ---------------------------------------------------------------------------
// File-scope state (cl_demo.c:37, :584)
// ---------------------------------------------------------------------------

/// `cl_demo.c:37` -- `static char name[MAX_OSPATH];`. Shared by
/// `CL_Record_f`, `CL_Resume_Record` and `CL_PlayDemo_f` exactly as in C:
/// `CL_Resume_Record` reopens whatever path the last `CL_Record_f` or
/// `CL_PlayDemo_f` left behind.
static mut NAME: [c_char; c::MAX_OSPATH] = [0; c::MAX_OSPATH];

/// `cl_demo.c:584` -- `static byte
/// weirdaltbufferthatprobablyisntneeded[NET_MAXMESSAGE];`.
static mut ALTBUF: [u8; NET_MAXMESSAGE] = [0; NET_MAXMESSAGE];

// ---------------------------------------------------------------------------
// Accessors
// ---------------------------------------------------------------------------

#[inline]
fn cl_p() -> *mut ClientState {
    ptr::addr_of_mut!(cl)
}

#[inline]
fn cls_p() -> *mut ClientStatic {
    ptr::addr_of_mut!(cls)
}

#[inline]
fn msg_p() -> *mut c::sizebuf_t {
    ptr::addr_of_mut!(c::net_message)
}

/// `cls.demofile` as the `FILE *` the stdio calls want.
///
/// # Safety
/// `cls` has process lifetime.
#[inline]
unsafe fn demofile() -> *mut c::FILE {
    // SAFETY: `cls` is a live, process-lifetime object.
    unsafe { (*cls_p()).demofile.cast::<c::FILE>() }
}

/// `Quake/q_minmax.h:49` `clamp_f`. `CLAMP (0.f, x, 255.f)` dispatches to the
/// `float` instantiation.
fn clamp_f(minval: c_float, val: c_float, maxval: c_float) -> c_float {
    if val < minval {
        minval
    } else if val > maxval {
        maxval
    } else {
        val
    }
}

// ---------------------------------------------------------------------------
// Batched net_message writer (ADR-009)
// ---------------------------------------------------------------------------

const W_BYTE: c_int = 0;
const W_SHORT: c_int = 1;
const W_LONG: c_int = 2;
const W_FLOAT: c_int = 3;
const W_STRING: c_int = 4;
const W_COORD: c_int = 5;

struct Writer {
    ops: [g::ClDemoWriteOp; WRITE_BATCH],
    n: usize,
}

impl Writer {
    fn new() -> Self {
        Writer {
            ops: [g::ClDemoWriteOp {
                kind: 0,
                i: 0,
                f: 0.0,
                u: 0,
                p: ptr::null(),
            }; WRITE_BATCH],
            n: 0,
        }
    }

    unsafe fn flush(&mut self) -> Raise {
        if self.n == 0 {
            return 0;
        }
        let count = self.n;
        self.n = 0;
        // SAFETY: `ops[..count]` is initialised and every `p` pointer is still
        // live at this point.
        unsafe { g::ClDemo_Glue_WriteBatch(self.ops.as_ptr(), count as c_int) }
    }

    unsafe fn push(
        &mut self,
        kind: c_int,
        i: c_int,
        f: c_float,
        u: c_uint,
        p: *const c_void,
    ) -> Raise {
        if self.n == WRITE_BATCH {
            // SAFETY: see `flush`.
            let r = unsafe { self.flush() };
            if r != 0 {
                return r;
            }
        }
        self.ops[self.n] = g::ClDemoWriteOp { kind, i, f, u, p };
        self.n += 1;
        0
    }

    unsafe fn byte(&mut self, v: c_int) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_BYTE, v, 0.0, 0, ptr::null()) }
    }

    unsafe fn short(&mut self, v: c_int) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_SHORT, v, 0.0, 0, ptr::null()) }
    }

    unsafe fn long(&mut self, v: c_int) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_LONG, v, 0.0, 0, ptr::null()) }
    }

    unsafe fn float(&mut self, v: c_float) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_FLOAT, 0, v, 0, ptr::null()) }
    }

    /// `s` must stay live until the next flush.
    unsafe fn string(&mut self, s: *const c_char) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_STRING, 0, 0.0, 0, s.cast::<c_void>()) }
    }

    unsafe fn coord(&mut self, f: c_float, flags: c_uint) -> Raise {
        // SAFETY: see `push`.
        unsafe { self.push(W_COORD, 0, f, flags, ptr::null()) }
    }

    /// `net_message.cursize`, with all pending ops applied first.
    unsafe fn cursize(&mut self, out: &mut c_int) -> Raise {
        // SAFETY: `net_message` is live; the read follows a successful flush.
        unsafe {
            raise!(self.flush());
            *out = (*msg_p()).cursize;
        }
        0
    }
}

// ---------------------------------------------------------------------------
// cl_demo.c:59 -- CL_StopPlayback
// ---------------------------------------------------------------------------

/// # Safety
/// FFI entry point; single-threaded, like the C.
unsafe fn cl_stop_playback() {
    // SAFETY: `cls` has process lifetime and `cls.demofile` is a live stream
    // whenever `demoplayback` is set.
    unsafe {
        if !(*cls_p()).demoplayback {
            return;
        }

        c::stdio::fclose(demofile());
        (*cls_p()).demoplayback = false;
        (*cls_p()).demoseeking = false;
        (*cls_p()).demopaused = false;
        (*cls_p()).demofile = ptr::null_mut();
        (*cls_p()).state = CA_DISCONNECTED;
        (*cls_p()).demo_prespawn_end = 0;

        if (*cls_p()).timedemo {
            cl_finish_time_demo();
        }

        g::Harness_DemoEnded();
    }
}

// ---------------------------------------------------------------------------
// cl_demo.c:85 -- CL_WriteDemoMessage
// ---------------------------------------------------------------------------

/// # Safety
/// `cls.demofile` must be an open stream.
unsafe fn cl_write_demo_message() {
    // SAFETY: `cls.demofile` is open on every call site; `net_message.data`
    // holds at least `cursize` bytes.
    unsafe {
        #[cfg(feature = "net")]
        {
            let mut header = [0u8; 16];
            crate::net::quake_rs_demo_record_header(
                (*msg_p()).cursize,
                (*cl_p()).viewangles.as_ptr(),
                header.as_mut_ptr(),
            );
            c::stdio::fwrite(
                header.as_ptr().cast::<c_void>(),
                header.len(),
                1,
                demofile(),
            );
            c::stdio::fwrite(
                (*msg_p()).data.cast::<c_void>(),
                (*msg_p()).cursize as usize,
                1,
                demofile(),
            );
            g::fflush(demofile());
        }
        #[cfg(not(feature = "net"))]
        {
            // COMPAT: ADR-010 -- `LittleLong`/`LittleFloat` are byte-order
            // fixups, identity on the little-endian targets the engine
            // supports; `to_le()` reproduces both exactly.
            let len: c_int = (*msg_p()).cursize.to_le();
            c::stdio::fwrite(ptr::addr_of!(len).cast::<c_void>(), 4, 1, demofile());
            for i in 0..3usize {
                let f = c_float::from_bits((*cl_p()).viewangles[i].to_bits().to_le());
                c::stdio::fwrite(ptr::addr_of!(f).cast::<c_void>(), 4, 1, demofile());
            }
            c::stdio::fwrite(
                (*msg_p()).data.cast::<c_void>(),
                (*msg_p()).cursize as usize,
                1,
                demofile(),
            );
            g::fflush(demofile());
        }
    }
}

// ---------------------------------------------------------------------------
// cl_demo.c:111 -- CL_GetDemoMessage
// ---------------------------------------------------------------------------

/// # Safety
/// FFI entry point; single-threaded, like the C.
unsafe fn cl_get_demo_message() -> c_int {
    // SAFETY: `cl`/`cls`/`net_message` all have process lifetime and
    // `cls.demofile` is open whenever `demoplayback` is set.
    unsafe {
        if (*cls_p()).demopaused {
            return 0;
        }

        if (*cls_p()).signon == SIGNONS - 2 {
            (*cls_p()).demo_prespawn_end = c::Sys_ftell(demofile());
        }
        // decide if it is time to grab the next message
        else if (*cls_p()).signon == SIGNONS
        // always grab until fully connected
        {
            if (*cls_p()).timedemo {
                if c::host_framecount == (*cls_p()).td_lastframe {
                    return 0; // already read this frame's message
                }
                (*cls_p()).td_lastframe = c::host_framecount;
                // if this is the second frame, grab the real td_starttime
                // so the bogus time on the first frame doesn't count
                if c::host_framecount == (*cls_p()).td_startframe + 1 {
                    (*cls_p()).td_starttime = g::realtime as c_float;
                }
            } else if (*cls_p()).demoseeking {
                // feed a reasonable cl.time value for effects / centerprints
                (*cl_p()).time = (*cl_p()).mtime[0];
                if (*cl_p()).mtime[0] > (*cls_p()).seektime as f64 {
                    (*cls_p()).demoseeking = false;
                    return 0;
                }
            } else if (*cl_p()).time <= (*cl_p()).mtime[0] {
                // cl_demo.c:186 leaves `cl.time > 0 &&` commented out upstream.
                return 0; // don't need another message yet
            }
        } else if (*cls_p()).signon < SIGNONS - 2 {
            (*cls_p()).demo_prespawn_end = 0;
        }

        // get the next message
        #[cfg(feature = "net")]
        {
            // COMPAT (accepted divergence, mirrored from cl_demo.c:157-164):
            // the 16-byte header is read atomically, where C read the length
            // and then each viewangle separately. Only malformed demos
            // differ, and both builds stop playback.
            let mut header = [0u8; 16];

            if c::stdio::fread(
                header.as_mut_ptr().cast::<c_void>(),
                header.len(),
                1,
                demofile(),
            ) != 1
            {
                cl_stop_playback();
                return 0;
            }
            let m = (*cl_p()).mviewangles[0];
            (*cl_p()).mviewangles[1] = m;
            if crate::net::quake_rs_demo_parse_record_header(
                header.as_ptr(),
                ptr::addr_of_mut!((*msg_p()).cursize),
                (*cl_p()).mviewangles[0].as_mut_ptr(),
            ) == 0
            {
                c::Sys_Error(c"Demo message > MAX_MSGLEN".as_ptr());
            }
        }
        #[cfg(not(feature = "net"))]
        {
            if c::stdio::fread(
                ptr::addr_of_mut!((*msg_p()).cursize).cast::<c_void>(),
                4,
                1,
                demofile(),
            ) != 1
            {
                cl_stop_playback();
                return 0;
            }
            let m = (*cl_p()).mviewangles[0];
            (*cl_p()).mviewangles[1] = m;
            for i in 0..3usize {
                let mut f: c_float = 0.0;
                if c::stdio::fread(ptr::addr_of_mut!(f).cast::<c_void>(), 4, 1, demofile()) != 1 {
                    cl_stop_playback();
                    return 0;
                }
                // COMPAT: ADR-010 -- `LittleFloat`, identity on LE targets.
                (*cl_p()).mviewangles[0][i] = c_float::from_bits(f.to_bits().to_le());
            }

            (*msg_p()).cursize = (*msg_p()).cursize.to_le();
            if (*msg_p()).cursize > MAX_MSGLEN {
                c::Sys_Error(c"Demo message > MAX_MSGLEN".as_ptr());
            }
        }

        let r = c::stdio::fread(
            (*msg_p()).data.cast::<c_void>(),
            (*msg_p()).cursize as usize,
            1,
            demofile(),
        );
        if r != 1 {
            cl_stop_playback();
            return 0;
        }

        1
    }
}

// ---------------------------------------------------------------------------
// cl_demo.c:214 -- CL_Seek_f
// ---------------------------------------------------------------------------

/// # Safety
/// FFI entry point; single-threaded, like the C.
unsafe fn cl_seek_f() -> Raise {
    // SAFETY: `cl`/`cls` have process lifetime; `Cmd_Argv` returns a live
    // NUL-terminated buffer for an in-range index.
    unsafe {
        if g::cmd_source != SRC_COMMAND {
            return 0;
        }

        if c::Cmd_Argc() != 2 {
            c::Con_Printf(c"seek [+/-]<offset> : [relative] seek in demo\n".as_ptr());
            return 0;
        }

        if !(*cls_p()).demoplayback {
            c::Con_Printf(c"Not playing a demo.\n".as_ptr());
            return 0;
        }

        (*cls_p()).demopaused = false;
        (*cl_p()).paused = false;
        if (*cls_p()).demospeed == 0.0 {
            (*cls_p()).demospeed = 1.0;
        }

        let mut offset: c_float = 0.0;
        // COMPAT: C leaves `offset_seconds` uninitialised; it is read only on
        // the `ret == 2` path, where `sscanf` has written it.
        let mut offset_seconds: c_float = 0.0;
        let ret = g::sscanf(
            c::Cmd_Argv(1),
            c"%f:%f".as_ptr(),
            ptr::addr_of_mut!(offset),
            ptr::addr_of_mut!(offset_seconds),
        );
        if ret == 2 {
            offset = offset * 60.0
                + if offset > 0.0 {
                    offset_seconds
                } else {
                    -offset_seconds
                };
        }

        if ret == 0 {
            c::Con_Printf(
                c"Expected time format is seconds or mm:ss with optional +/- prefix.\n".as_ptr(),
            );
            c::Con_Printf(c"Examples:  12:34  +20  -3:15\n".as_ptr());
            return 0;
        }

        let relative = offset < 0.0 || *c::Cmd_Argv(1) == b'+' as c_char;
        (*cls_p()).seektime = if relative {
            ((*cl_p()).time + offset as f64) as c_float
        } else {
            offset
        };

        // large positive offsets could benefit from demoseeking, but we'd lose prints etc
        if (offset < 0.0 || (!relative && (offset as f64) < (*cl_p()).time))
            && (*cls_p()).demo_prespawn_end != 0
        {
            c::Sys_fseek(demofile(), (*cls_p()).demo_prespawn_end, SEEK_SET);
            (*cl_p()).mtime[0] = 0.0;
            (*cl_p()).time = 0.0;
            (*cls_p()).demoseeking = true;

            ptr::write_bytes(
                ptr::addr_of_mut!(cl_dlights).cast::<u8>(),
                0,
                size_of::<[c::cl_tent::dlight_t; MAX_DLIGHTS]>(),
            );
            ptr::write_bytes(
                ptr::addr_of_mut!(cl_temp_entities).cast::<u8>(),
                0,
                size_of::<[EntityOpaque; MAX_TEMP_ENTITIES]>(),
            );
            ptr::write_bytes(
                ptr::addr_of_mut!(c::cl_tent::cl_beams).cast::<u8>(),
                0,
                size_of::<[c::cl_tent::beam_t; MAX_BEAMS]>(),
            );
            raise!(g::ClDemo_Glue_SeekEffects());
            if (*cl_p()).intermission != 0 {
                (*cl_p()).intermission = 0;
                raise!(g::ClDemo_Glue_BgmStop());
            }
            ptr::write_bytes((*cl_p()).stats.as_mut_ptr(), 0, MAX_CL_STATS);
            ptr::write_bytes((*cl_p()).statsf.as_mut_ptr(), 0, MAX_CL_STATS);

            // replay last signon for stats and lightstyles
            (*cls_p()).signon = SIGNONS - 2;
            raise!(g::ClDemo_Glue_StopAllSounds());
        } else {
            (*cl_p()).time = (*cls_p()).seektime as f64;
        }

        g::scr_clock_off = 2.5; // show clock for a few seconds after a seek
        0
    }
}

// ---------------------------------------------------------------------------
// cl_demo.c:293 -- CL_GetMessage
// ---------------------------------------------------------------------------

/// # Safety
/// FFI entry point; single-threaded, like the C.
unsafe fn cl_get_message(out: &mut c_int) -> Raise {
    // SAFETY: `cls`/`net_message` have process lifetime.
    unsafe {
        if (*cls_p()).demoplayback {
            *out = cl_get_demo_message();
            return 0;
        }

        let mut r: c_int = 0;
        loop {
            raise!(g::ClDemo_Glue_NetGetMessage(
                (*cls_p()).netcon.cast::<c::qsocket_s>(),
                ptr::addr_of_mut!(r)
            ));

            if r != 1 && r != 2 {
                *out = r;
                return 0;
            }

            // discard nop keepalive message
            if (*msg_p()).cursize == 1 && *(*msg_p()).data == SVC_NOP {
                c::Con_Printf(c"<-- server to client keepalive\n".as_ptr());
            } else {
                break;
            }
        }

        if (*cls_p()).demorecording {
            cl_write_demo_message();
        }

        *out = r;
        0
    }
}

// ---------------------------------------------------------------------------
// cl_demo.c:327 -- CL_Stop_f
// ---------------------------------------------------------------------------

/// # Safety
/// FFI entry point; single-threaded, like the C.
unsafe fn cl_stop_f() -> Raise {
    // SAFETY: `cls`/`net_message` have process lifetime.
    unsafe {
        if g::cmd_source != SRC_COMMAND {
            return 0;
        }

        if !(*cls_p()).demorecording {
            c::Con_Printf(c"Not recording a demo.\n".as_ptr());
            return 0;
        }

        // write a disconnect message to the demo file
        c::cvar_cmd::SZ_Clear(msg_p());
        let mut wr = Writer::new();
        raise!(wr.byte(SVC_DISCONNECT));
        raise!(wr.flush());
        cl_write_demo_message();

        // finish up
        c::stdio::fclose(demofile());
        (*cls_p()).demofile = ptr::null_mut();
        (*cls_p()).demorecording = false;
        c::Con_Printf(c"Completed demo\n".as_ptr());

        // ericw -- update demo tab-completion list
        raise!(g::ClDemo_Glue_DemoListRebuild());
        0
    }
}

// ---------------------------------------------------------------------------
// cl_demo.c:353 -- CL_Record_Serverdata
// ---------------------------------------------------------------------------

/// # Safety
/// `cls.demofile` must be open and `net_message` swapped to the alt buffer.
unsafe fn cl_record_serverdata() -> Raise {
    // SAFETY: the precache tables are NUL-pointer terminated and `cl` has
    // process lifetime.
    unsafe {
        let mut wr = Writer::new();
        raise!(wr.byte(SVC_SERVERINFO));
        if (*cl_p()).protocol_pext2 != 0 {
            raise!(wr.long(PROTOCOL_FTE_PEXT2));
            raise!(wr.long((*cl_p()).protocol_pext2 as c_int));
        }
        raise!(wr.long((*cl_p()).protocol as c_int));
        if (*cl_p()).protocol == PROTOCOL_RMQ {
            raise!(wr.long((*cl_p()).protocolflags as c_int));
        }
        if (*cl_p()).protocol_pext2 & PEXT2_PREDINFO != 0 {
            raise!(wr.string(c::COM_SkipPath(
                ptr::addr_of!(c::com_gamedir).cast::<c_char>()
            )));
        }
        raise!(wr.byte((*cl_p()).maxclients));
        raise!(wr.byte((*cl_p()).gametype));
        raise!(wr.string((*cl_p()).levelname.as_ptr()));
        let mut i = 1usize;
        while !(*cl_p()).model_precache[i].is_null() {
            raise!(wr.string((*(*cl_p()).model_precache[i]).name.as_ptr()));
            i += 1;
        }
        raise!(wr.byte(0));
        let mut i = 1usize;
        // FIXME: might not send any if nosound is set
        while !(*cl_p()).sound_precache[i].is_null() {
            raise!(wr.string((*(*cl_p()).sound_precache[i]).name.as_ptr()));
            i += 1;
        }
        raise!(wr.byte(0));
        // FIXME: cd track (current rather than initial?)
        // FIXME: initial view entity (for clients that don't want to mess up scoreboards)
        raise!(wr.byte(SVC_SIGNONNUM));
        raise!(wr.byte(1));
        raise!(wr.flush());
        cl_write_demo_message();
        c::cvar_cmd::SZ_Clear(msg_p());
        0
    }
}

// ---------------------------------------------------------------------------
// cl_demo.c:385 -- CL_Record_Prespawn
// ---------------------------------------------------------------------------

/// # Safety
/// `cls.demofile` must be open and `net_message` swapped to the alt buffer.
unsafe fn cl_record_prespawn() -> Raise {
    // SAFETY: `cl.entities` / `cl.static_entities` hold `cl.num_entities` /
    // `cl.num_statics` live slots; `snd_channels` is a fixed C array.
    unsafe {
        let mut wr = Writer::new();
        let mut cursize: c_int = 0;

        // baselines
        for idx in 0..(*cl_p()).num_entities {
            let ent = ((*cl_p()).entities.cast::<Entity>()).add(idx as usize);
            let state = ptr::addr_of_mut!((*ent).baseline);
            if bytes_equal(
                state.cast::<u8>(),
                ptr::addr_of!(nullentitystate).cast::<u8>(),
                size_of::<EntityState>(),
            ) {
                continue; // no need
            }
            raise!(wr.flush());
            raise!(g::ClDemo_Glue_WriteStaticOrBaseLine(
                idx,
                state.cast::<c_void>(),
                (*cl_p()).protocol_pext2,
                (*cl_p()).protocol,
                (*cl_p()).protocolflags
            ));

            raise!(wr.cursize(&mut cursize));
            if cursize > 4096 {
                // periodically flush so that large maps don't need larger than vanilla limits
                cl_write_demo_message();
                c::cvar_cmd::SZ_Clear(msg_p());
            }
        }

        // static ents
        for idx in 1..(*cl_p()).num_statics {
            let ent = (*(*cl_p()).static_entities.add(idx as usize)).cast::<Entity>();
            raise!(wr.flush());
            raise!(g::ClDemo_Glue_WriteStaticOrBaseLine(
                -1,
                ptr::addr_of_mut!((*ent).baseline).cast::<c_void>(),
                (*cl_p()).protocol_pext2,
                (*cl_p()).protocol,
                (*cl_p()).protocolflags
            ));

            raise!(wr.cursize(&mut cursize));
            if cursize > 4096 {
                cl_write_demo_message();
                c::cvar_cmd::SZ_Clear(msg_p());
            }
        }

        // static sounds
        for i in NUM_AMBIENTS..c::total_channels {
            let ss = ptr::addr_of_mut!(c::snd_channels)
                .cast::<c::channel_t>()
                .add(i as usize);

            if (*ss).sfx.is_null() {
                continue;
            }
            if (*ss).entnum != 0 || (*ss).entchannel != 0 {
                continue; // can't have been a static sound
            }
            let mut sc: *mut c_void = ptr::null_mut();
            raise!(wr.flush());
            raise!(g::ClDemo_Glue_LoadSound(
                (*ss).sfx.cast::<c_void>(),
                ptr::addr_of_mut!(sc)
            ));
            let sc = sc.cast::<c::sfxcache_t>();
            if sc.is_null() || (*sc).loopstart == -1 {
                continue; // can't have been a (valid) static sound
            }

            let mut idx = 1;
            while idx < MAX_SOUNDS && !(*cl_p()).sound_precache[idx as usize].is_null() {
                if ptr::eq(
                    (*cl_p()).sound_precache[idx as usize].cast::<c_void>(),
                    (*ss).sfx.cast::<c_void>(),
                ) {
                    break;
                }
                idx += 1;
            }
            if idx == MAX_SOUNDS {
                continue; // can't figure out which sound it was
            }

            raise!(wr.byte(if idx > 255 {
                SVC_SPAWNSTATICSOUND2
            } else {
                SVC_SPAWNSTATICSOUND
            }));
            raise!(wr.coord((*ss).origin[0], (*cl_p()).protocolflags));
            raise!(wr.coord((*ss).origin[1], (*cl_p()).protocolflags));
            raise!(wr.coord((*ss).origin[2], (*cl_p()).protocolflags));
            if idx > 255 {
                raise!(wr.short(idx));
            } else {
                raise!(wr.byte(idx));
            }
            raise!(wr.byte((*ss).master_vol));
            // COMPAT: ADR-010 -- `ss->dist_mult * 1000 * 64` is two float
            // multiplies in source order, then a C truncating cast.
            raise!(wr.byte(clamp_f(0.0, ((*ss).dist_mult * 1000.0) * 64.0, 255.0) as c_int));

            raise!(wr.cursize(&mut cursize));
            if cursize > 4096 {
                cl_write_demo_message();
                c::cvar_cmd::SZ_Clear(msg_p());
            }
        }

        // particleindexes
        for idx in 0..MAX_PARTICLETYPES {
            if (*cl_p()).particle_precache[idx].name.is_null() {
                continue;
            }
            raise!(wr.byte(SVCDP_PRECACHE));
            raise!(wr.short(0x4000 | idx as c_int));
            raise!(wr.string((*cl_p()).particle_precache[idx].name));

            raise!(wr.cursize(&mut cursize));
            if cursize > 4096 {
                cl_write_demo_message();
                c::cvar_cmd::SZ_Clear(msg_p());
            }
        }

        raise!(wr.byte(SVC_SIGNONNUM));
        raise!(wr.byte(2));
        raise!(wr.flush());
        cl_write_demo_message();
        c::cvar_cmd::SZ_Clear(msg_p());
        0
    }
}

/// `memcmp (a, b, n) == 0`.
///
/// # Safety
/// Both pointers must be readable for `n` bytes.
unsafe fn bytes_equal(a: *const u8, b: *const u8, n: usize) -> bool {
    // SAFETY: caller guarantees both ranges.
    unsafe { core::slice::from_raw_parts(a, n) == core::slice::from_raw_parts(b, n) }
}

// ---------------------------------------------------------------------------
// cl_demo.c:478 -- CL_Record_Spawn
// ---------------------------------------------------------------------------

/// # Safety
/// `cls.demofile` must be open and `net_message` swapped to the alt buffer.
unsafe fn cl_record_spawn() -> Raise {
    // SAFETY: `cl.scores` holds `cl.maxclients` slots; `cl_lightstyle` is a
    // fixed C array.
    unsafe {
        let mut wr = Writer::new();
        let mut cursize: c_int = 0;

        // player names, colors, and frag counts
        for i in 0..(*cl_p()).maxclients {
            let sc = (*cl_p()).scores.add(i as usize);
            raise!(wr.byte(SVC_UPDATENAME));
            raise!(wr.byte(i));
            raise!(wr.string((*sc).name.as_ptr()));
            raise!(wr.byte(SVC_UPDATEFRAGS));
            raise!(wr.byte(i));
            raise!(wr.short((*sc).frags));
            raise!(wr.byte(SVC_UPDATECOLORS));
            raise!(wr.byte(i));
            raise!(wr.byte((*sc).colors));
        }

        // send all current light styles
        for i in 0..MAX_LIGHTSTYLES {
            let ls = ptr::addr_of_mut!(c::cl_parse::cl_lightstyle)
                .cast::<c::cl_parse::lightstyle_t>()
                .add(i);
            if (*ls).map[0] != 0 {
                raise!(wr.byte(SVC_LIGHTSTYLE));
                raise!(wr.byte(i as c_int));
                raise!(wr.string((*ls).map.as_ptr()));
            }

            raise!(wr.cursize(&mut cursize));
            if cursize > 4096 {
                cl_write_demo_message();
                c::cvar_cmd::SZ_Clear(msg_p());
            }
        }

        // what about the current CD track... future consideration.

        let mut fog_cmd: *const c_char = ptr::null();
        raise!(wr.flush());
        raise!(g::ClDemo_Glue_FogGetFogCommand(ptr::addr_of_mut!(fog_cmd)));
        if !fog_cmd.is_null() {
            raise!(wr.byte(SVC_STUFFTEXT));
            raise!(wr.string(fog_cmd));
        }

        let mut sky_cmd: *const c_char = ptr::null();
        raise!(wr.flush());
        raise!(g::ClDemo_Glue_SkyGetSkyCommand(ptr::addr_of_mut!(sky_cmd)));
        if !sky_cmd.is_null() {
            raise!(wr.byte(SVC_STUFFTEXT));
            raise!(wr.string(sky_cmd));
        }

        // stats
        for i in 0..MAX_CL_STATS {
            if (*cl_p()).stats[i] == 0 && (*cl_p()).statsf[i] == 0.0 {
                continue;
            }

            raise!(wr.cursize(&mut cursize));
            if cursize > 4096 {
                cl_write_demo_message();
                c::cvar_cmd::SZ_Clear(msg_p());
            }

            if (*cl_p()).stats[i] as f64 != (*cl_p()).statsf[i] as f64
                && ((*cl_p()).stats[i] as c_uint) <= 0x00ff_ffff
            {
                // if the float representation seems to have more precision then use that, unless
                // its getting huge in which case we're probably getting fpu truncation, so go back
                // to more compatible ints
                raise!(wr.byte(SVCFTE_UPDATESTATFLOAT));
                raise!(wr.byte(i as c_int));
                raise!(wr.float((*cl_p()).statsf[i]));
            } else if (*cl_p()).stats[i] >= 0
                && (*cl_p()).stats[i] <= 255
                && ((*cl_p()).protocol_pext2 & PEXT2_PREDINFO) != 0
            {
                raise!(wr.byte(SVCDP_UPDATESTATBYTE));
                raise!(wr.byte(i as c_int));
                raise!(wr.byte((*cl_p()).stats[i]));
            } else {
                raise!(wr.byte(SVC_UPDATESTAT));
                raise!(wr.byte(i as c_int));
                raise!(wr.long((*cl_p()).stats[i]));
            }
        }

        // view entity
        raise!(wr.byte(SVC_SETVIEW));
        raise!(wr.short((*cl_p()).viewentity));

        // signon
        raise!(wr.byte(SVC_SIGNONNUM));
        raise!(wr.byte(3));

        raise!(wr.flush());
        cl_write_demo_message();
        c::cvar_cmd::SZ_Clear(msg_p());

        // ask the server to reset entity deltas. yes this means playback will wait a couple of
        // frames before it actually starts playing but oh well.
        if (*cl_p()).protocol_pext2 & PEXT2_REPLACEMENTDELTAS != 0 {
            (*cl_p()).ackframes_count = 0;
            (*cl_p()).ackframes[(*cl_p()).ackframes_count as usize] = -1;
            (*cl_p()).ackframes_count += 1;
        }
        0
    }
}

// ---------------------------------------------------------------------------
// cl_demo.c:581 -- CL_Record_Signons
// ---------------------------------------------------------------------------

/// # Safety
/// `cls.demofile` must be open.
unsafe fn cl_record_signons() -> Raise {
    // SAFETY: `net_message` has process lifetime; `ALTBUF` outlives the swap.
    unsafe {
        let data = (*msg_p()).data;
        let cursize = (*msg_p()).cursize;

        (*msg_p()).data = ptr::addr_of_mut!(ALTBUF).cast::<u8>();
        c::cvar_cmd::SZ_Clear(msg_p());

        let r = cl_record_signons_body();

        // restore net_message
        (*msg_p()).data = data;
        (*msg_p()).cursize = cursize;
        r
    }
}

/// The three record passes, split out so the `net_message` restore in
/// [`cl_record_signons`] runs on the raise path too. C could not raise out of
/// this region at all -- the whole point of the ADR-009 status discipline is
/// that a guard status unwinds by `return`, so the restore has to be explicit.
///
/// # Safety
/// See [`cl_record_signons`].
unsafe fn cl_record_signons_body() -> Raise {
    // SAFETY: see the caller.
    unsafe {
        raise!(cl_record_serverdata());
        raise!(cl_record_prespawn());
        raise!(cl_record_spawn());
        0
    }
}

// ---------------------------------------------------------------------------
// cl_demo.c:609 -- CL_Record_f
// ---------------------------------------------------------------------------

/// # Safety
/// FFI entry point; single-threaded, like the C.
unsafe fn cl_record_f() -> Raise {
    // SAFETY: `cl`/`cls` have process lifetime; `Cmd_Argv` returns a live
    // NUL-terminated buffer for an in-range index.
    unsafe {
        let mut relname = [0 as c_char; c::MAX_OSPATH];

        let track: c_int;

        if g::cmd_source != SRC_COMMAND {
            return 0;
        }

        if (*cls_p()).demoplayback {
            c::Con_Printf(c"Can't record during demo playback\n".as_ptr());
            return 0;
        }

        if (*cls_p()).demorecording {
            raise!(cl_stop_f());
        }

        let c_argc = c::Cmd_Argc();
        if c_argc != 2 && c_argc != 3 && c_argc != 4 {
            c::Con_Printf(c"record <demoname> [<map> [cd track]]\n".as_ptr());
            return 0;
        }

        if !g::strstr(c::Cmd_Argv(1), c"..".as_ptr()).is_null() {
            c::Con_Printf(c"Relative pathnames are not allowed.\n".as_ptr());
            return 0;
        }

        if c_argc == 2 && (*cls_p()).state == CA_CONNECTED {
            if (*cls_p()).signon < 2 {
                c::Con_Printf(c"Can't record - try again when connected\n".as_ptr());
                return 0;
            }
            match (*cl_p()).protocol {
                PROTOCOL_NETQUAKE | PROTOCOL_FITZQUAKE | PROTOCOL_RMQ => {}
                _ => {
                    c::Con_Printf(c"Can not record - protocol not supported for recording mid-map\nClient demo recording must be started before connecting\n".as_ptr());
                    return 0;
                }
            }
        }

        // write the forced cd track number, or -1
        if c_argc == 4 {
            track = g::atoi(c::Cmd_Argv(3));
            // COMPAT: the C prints cls.forcetrack, which still holds the
            // PREVIOUS recording's track here -- `track` is not stored until
            // :709. Preserved bug-for-bug (cl_demo.c:669).
            c::Con_Printf(c"Forcing CD track to %i\n".as_ptr(), (*cls_p()).forcetrack);
        } else {
            track = -1;
        }

        // save the demo name here, before potentially loading a new map (which would change argv[1])
        g::q_strlcpy(relname.as_mut_ptr(), c::Cmd_Argv(1), relname.len());

        // start the map up
        if c_argc > 2 {
            let text = g::va(c"map %s".as_ptr(), c::Cmd_Argv(2));
            raise!(g::ClDemo_Glue_CmdExecuteString(text, SRC_COMMAND as c_int));
            if (*cls_p()).state != CA_CONNECTED {
                return 0;
            }
        }

        // open the demo file
        c::COM_AddExtension(relname.as_mut_ptr(), c".dem".as_ptr(), relname.len());
        g::q_snprintf(
            ptr::addr_of_mut!(NAME).cast::<c_char>(),
            c::MAX_OSPATH,
            c"%s/%s".as_ptr(),
            ptr::addr_of!(c::com_gamedir).cast::<c_char>(),
            relname.as_ptr(),
        );

        c::Con_SafePrintf(c"Recording to ".as_ptr());
        g::Con_LinkPrintf(
            ptr::addr_of!(NAME).cast::<c_char>(),
            c"%s".as_ptr(),
            relname.as_ptr(),
        );
        c::Con_SafePrintf(c".\n".as_ptr());

        (*cls_p()).demofile =
            c::Sys_fopen(ptr::addr_of!(NAME).cast::<c_char>(), c"wb".as_ptr()).cast::<c_void>();
        if (*cls_p()).demofile.is_null() {
            c::Con_Printf(
                c"ERROR: couldn't create %s\n".as_ptr(),
                ptr::addr_of!(NAME).cast::<c_char>(),
            );
            return 0;
        }

        (*cls_p()).forcetrack = track;
        #[cfg(feature = "net")]
        {
            let mut trackline = [0 as c_char; 32];
            let tracklen = crate::net::quake_rs_demo_forcetrack_line(
                (*cls_p()).forcetrack,
                trackline.as_mut_ptr(),
                trackline.len() as c_int,
            );
            c::stdio::fwrite(
                trackline.as_ptr().cast::<c_void>(),
                tracklen as usize,
                1,
                demofile(),
            );
        }
        #[cfg(not(feature = "net"))]
        {
            c::cvar_cmd::fprintf(demofile(), c"%i\n".as_ptr(), (*cls_p()).forcetrack);
        }

        (*cls_p()).demorecording = true;

        // from ProQuake: initialize the demo file if we're already connected
        if c_argc == 2 && (*cls_p()).state == CA_CONNECTED {
            raise!(cl_record_signons());
        }
        0
    }
}

// ---------------------------------------------------------------------------
// cl_demo.c:726 -- CL_Resume_Record
// ---------------------------------------------------------------------------

/// # Safety
/// FFI entry point; single-threaded, like the C.
unsafe fn cl_resume_record(recordsignons: bool) -> Raise {
    // SAFETY: `cls` has process lifetime; `NAME` is NUL-terminated.
    unsafe {
        (*cls_p()).demofile =
            c::Sys_fopen(ptr::addr_of!(NAME).cast::<c_char>(), c"r+b".as_ptr()).cast::<c_void>();
        if (*cls_p()).demofile.is_null() {
            c::Con_Printf(
                c"ERROR: couldn't append to %s - recording stopped\n".as_ptr(),
                ptr::addr_of!(NAME).cast::<c_char>(),
            );
            return 0;
        }
        // overwrite svc_disconnect
        #[cfg(feature = "net")]
        {
            c::Sys_fseek(
                demofile(),
                crate::net::quake_rs_demo_resume_seek_offset(),
                SEEK_END,
            );
        }
        #[cfg(not(feature = "net"))]
        {
            c::Sys_fseek(demofile(), -17, SEEK_END);
        }
        c::Con_Printf(c"Demo recording resumed\n".as_ptr());
        (*cls_p()).demorecording = true;
        if recordsignons {
            raise!(cl_record_signons());
        }
        0
    }
}

// ---------------------------------------------------------------------------
// cl_demo.c:753 -- CL_PlayDemo_f
// ---------------------------------------------------------------------------

/// # Safety
/// FFI entry point; single-threaded, like the C.
unsafe fn cl_play_demo_f() -> Raise {
    // SAFETY: `cls` has process lifetime; `Cmd_Argv` returns a live
    // NUL-terminated buffer for an in-range index.
    unsafe {
        let invalid: bool;

        if g::cmd_source != SRC_COMMAND {
            return 0;
        }

        if c::Cmd_Argc() != 2 {
            c::Con_Printf(c"playdemo <demoname> : plays a demo\n".as_ptr());
            return 0;
        }

        // disconnect from server
        raise!(g::ClDemo_Glue_Disconnect());

        // open the demo file
        g::q_strlcpy(
            ptr::addr_of_mut!(NAME).cast::<c_char>(),
            c::Cmd_Argv(1),
            c::MAX_OSPATH,
        );
        c::COM_AddExtension(
            ptr::addr_of_mut!(NAME).cast::<c_char>(),
            c".dem".as_ptr(),
            c::MAX_OSPATH,
        );

        c::Con_Printf(
            c"Playing demo from %s.\n".as_ptr(),
            ptr::addr_of!(NAME).cast::<c_char>(),
        );

        let mut file: *mut c::FILE = ptr::null_mut();
        raise!(g::ClDemo_Glue_ComFOpenFile(
            ptr::addr_of!(NAME).cast::<c_char>(),
            ptr::addr_of_mut!(file)
        ));
        (*cls_p()).demofile = file.cast::<c_void>();
        if (*cls_p()).demofile.is_null() {
            c::Con_Printf(
                c"ERROR: couldn't open %s\n".as_ptr(),
                ptr::addr_of!(NAME).cast::<c_char>(),
            );
            (*cls_p()).demonum = -1; // stop demo loop
            return 0;
        }

        // ZOID, fscanf is evil
        // O.S.: if a space character e.g. 0x20 (' ') follows '\n',
        // fscanf skips that byte too and screws up further reads.
        #[cfg(feature = "net")]
        {
            // COMPAT (mirrored from cl_demo.c:797-800): fscanf's
            // whitespace/digit runs were unbounded; this reads a 64-byte
            // chunk, so a hand-authored header line longer than that is
            // rejected as invalid where the C build would accept it.
            let mut trackline = [0 as c_char; 64];
            let mut consumed: c_int = 0;
            let linestart = c::Sys_ftell(demofile());
            let got = c::stdio::fread(
                trackline.as_mut_ptr().cast::<c_void>(),
                1,
                trackline.len(),
                demofile(),
            ) as c_int;
            invalid = crate::net::quake_rs_demo_parse_forcetrack(
                trackline.as_ptr(),
                got,
                ptr::addr_of_mut!((*cls_p()).forcetrack),
                ptr::addr_of_mut!(consumed),
            ) == 0;
            if !invalid {
                c::Sys_fseek(demofile(), linestart + consumed as i64, SEEK_SET);
            }
        }
        #[cfg(not(feature = "net"))]
        {
            invalid = g::fscanf(
                demofile(),
                c"%i".as_ptr(),
                ptr::addr_of_mut!((*cls_p()).forcetrack),
            ) != 1
                || c::stdio::fgetc(demofile()) != b'\n' as c_int;
        }
        if invalid {
            c::stdio::fclose(demofile());
            (*cls_p()).demofile = ptr::null_mut();
            (*cls_p()).demonum = -1; // stop demo loop
            c::Con_Printf(
                c"ERROR: demo \"%s\" is invalid\n".as_ptr(),
                ptr::addr_of!(NAME).cast::<c_char>(),
            );
            return 0;
        }

        (*cls_p()).demoplayback = true;
        (*cls_p()).demopaused = false;
        (*cls_p()).demospeed = 1.0;
        (*cls_p()).state = CA_CONNECTED;

        // get rid of the menu and/or console
        g::key_dest = KEY_GAME;
        0
    }
}

// ---------------------------------------------------------------------------
// cl_demo.c:828 -- CL_FinishTimeDemo
// ---------------------------------------------------------------------------

/// # Safety
/// FFI entry point; single-threaded, like the C.
unsafe fn cl_finish_time_demo() {
    // SAFETY: `cls` has process lifetime.
    unsafe {
        (*cls_p()).timedemo = false;

        // the first frame didn't count
        let frames = (c::host_framecount - (*cls_p()).td_startframe) - 1;
        // COMPAT: ADR-010 -- `realtime` is `double` and `td_starttime` is
        // `float`; the subtraction happens in double and the result is stored
        // to a `float`, exactly as C does.
        let mut time = (g::realtime - (*cls_p()).td_starttime as f64) as c_float;
        if time == 0.0 {
            time = 1.0;
        }
        // ADR-005: `%f` only; no `%g`/`%e` reaches the formatter here.
        c::Con_Printf(
            c"%i frames %5.1f seconds %5.1f fps\n".as_ptr(),
            frames,
            time as f64,
            (frames as c_float / time) as f64,
        );
    }
}

// ---------------------------------------------------------------------------
// cl_demo.c:850 -- CL_TimeDemo_f
// ---------------------------------------------------------------------------

/// # Safety
/// FFI entry point; single-threaded, like the C.
unsafe fn cl_time_demo_f() -> Raise {
    // SAFETY: `cls` has process lifetime.
    unsafe {
        if g::cmd_source != SRC_COMMAND {
            return 0;
        }

        if c::Cmd_Argc() != 2 {
            c::Con_Printf(c"timedemo <demoname> : gets demo speeds\n".as_ptr());
            return 0;
        }

        raise!(cl_play_demo_f());
        if (*cls_p()).demofile.is_null() {
            return 0;
        }

        // cls.td_starttime will be grabbed at the second frame of the demo, so
        // all the loading time doesn't get counted

        (*cls_p()).timedemo = true;
        (*cls_p()).td_startframe = c::host_framecount;
        (*cls_p()).td_lastframe = -1; // get a new message this frame
        0
    }
}

// ---------------------------------------------------------------------------
// ADR-009 status cores. Each returns a Host_Guard status verbatim; only
// Quake/cl_demo_glue.c re-issues the jump.
// ---------------------------------------------------------------------------

/// # Safety
/// FFI entry point.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_stop_playback() {
    // SAFETY: see `cl_stop_playback`.
    unsafe { cl_stop_playback() }
}

/// # Safety
/// FFI entry point.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_seek_f() -> Raise {
    // SAFETY: see `cl_seek_f`.
    unsafe { cl_seek_f() }
}

/// # Safety
/// `out` must point at a writable `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_get_message(out: *mut c_int) -> Raise {
    // SAFETY: `out` is a live `int` supplied by `cl_demo_glue.c`.
    unsafe {
        let mut v: c_int = 0;
        let r = cl_get_message(&mut v);
        *out = v;
        r
    }
}

/// # Safety
/// FFI entry point.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_stop_f() -> Raise {
    // SAFETY: see `cl_stop_f`.
    unsafe { cl_stop_f() }
}

/// # Safety
/// FFI entry point.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_record_f() -> Raise {
    // SAFETY: see `cl_record_f`.
    unsafe { cl_record_f() }
}

/// # Safety
/// FFI entry point.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_resume_record(recordsignons: bool) -> Raise {
    // SAFETY: see `cl_resume_record`.
    unsafe { cl_resume_record(recordsignons) }
}

/// # Safety
/// FFI entry point.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_play_demo_f() -> Raise {
    // SAFETY: see `cl_play_demo_f`.
    unsafe { cl_play_demo_f() }
}

/// # Safety
/// FFI entry point.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_cl_time_demo_f() -> Raise {
    // SAFETY: see `cl_time_demo_f`.
    unsafe { cl_time_demo_f() }
}
