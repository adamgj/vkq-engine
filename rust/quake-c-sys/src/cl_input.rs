//! `Quake/cl_input_glue.c` declarations (Rust migration Phase 7 M7, T7.2).
//!
//! ADR-011: engine C symbols are declared only in this crate. The glue file
//! owns the C-visible objects `Quake/cl_input.c` used to define -- the
//! seventeen `kbutton_t` input states plus `in_impulse` (`cl_input.c:53-59`)
//! and the nine movement cvars (`:323-335`) -- plus the two `Host_Guard`
//! trampolines this port needs (ADR-009).
//!
//! The `kbutton_t` objects and the cvars stay C-visible because still-C
//! translation units read them: `in_sdl.c:553` and `:600-622` read
//! `in_strafe`/`in_speed`/`in_mlook`/`in_klook` and four of the cvars,
//! `menu.c` and `view.c` read three more, and `cl_main.c:1376-1390` is what
//! actually registers all nine (registration order is observable in
//! `config.cfg`, so it stays where it is).
//!
//! `cl`/`cls` were ADR-007 dual-view rows, C-owned for T7.2; the row closed in
//! T7.4 and `quake-capi/src/cl_main.rs` now owns the storage. They are
//! mirror-typed, so -- as
//! `quake-c-sys/src/sv_user.rs` records -- they are declared in
//! `quake-capi/src/cl_input.rs`, which can name `quake_types`; this crate has
//! no `[dependencies]`.

use core::ffi::{c_float, c_int, c_uint};

/// `Quake/client.h:374-377` -- `typedef struct {int down[2]; int state;}
/// kbutton_t;`. Small enough to mirror; `quake-c-sys` cannot depend on
/// `quake-types`, so the mirror lives next to the externs that need it.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_camel_case_types)]
pub struct kbutton_t {
    pub down: [c_int; 2],
    pub state: c_int,
}

/// One buffered `MSG_Write*` for [`ClInput_Glue_WriteBatch`]. `u` carries the
/// protocol flag word per op rather than being re-read on the C side, so the
/// batch stays a pure replay of what Rust decided.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClInputWriteOp {
    pub kind: c_int,
    pub i: c_int,
    pub f: c_float,
    pub u: c_uint,
}

extern "C" {
    /* Quake/cl_input_glue.c data (cl_input.c:53-59, :323-335) */

    pub static mut in_mlook: kbutton_t;
    pub static mut in_klook: kbutton_t;
    pub static mut in_left: kbutton_t;
    pub static mut in_right: kbutton_t;
    pub static mut in_forward: kbutton_t;
    pub static mut in_back: kbutton_t;
    pub static mut in_lookup: kbutton_t;
    pub static mut in_lookdown: kbutton_t;
    pub static mut in_moveleft: kbutton_t;
    pub static mut in_moveright: kbutton_t;
    pub static mut in_strafe: kbutton_t;
    pub static mut in_speed: kbutton_t;
    pub static mut in_use: kbutton_t;
    pub static mut in_jump: kbutton_t;
    pub static mut in_attack: kbutton_t;
    pub static mut in_up: kbutton_t;
    pub static mut in_down: kbutton_t;

    pub static mut in_impulse: c_int;

    pub static mut cl_upspeed: crate::cvar_t;
    pub static mut cl_forwardspeed: crate::cvar_t;
    pub static mut cl_backspeed: crate::cvar_t;
    pub static mut cl_sidespeed: crate::cvar_t;
    pub static mut cl_movespeedkey: crate::cvar_t;
    pub static mut cl_yawspeed: crate::cvar_t;
    pub static mut cl_pitchspeed: crate::cvar_t;
    pub static mut cl_anglespeedkey: crate::cvar_t;
    pub static mut cl_alwaysrun: crate::cvar_t;

    /* Quake/cl_input_glue.c guards -- each returns a Host_Guard status */

    /// Replays a run of buffered `MSG_Write*` calls into `sb` inside one
    /// guarded frame. Every `MSG_Write*` reaches `SZ_GetSpace`
    /// (`net_msg.c:479`), which `Host_Error`s when the sizebuf disallows
    /// overflow, so none of them may be called from a Rust frame (ADR-009).
    pub fn ClInput_Glue_WriteBatch(
        sb: *mut crate::sizebuf_t,
        ops: *const ClInputWriteOp,
        count: c_int,
    ) -> c_int;

    /// `CL_Disconnect ()` (`cl_input.c:558`), on a lost server connection. A
    /// raise site twice over: its own `MSG_WriteByte` (`cl_main.c:391`) and,
    /// with a local server running, `Host_ShutdownServer` -> `SV_DropClient`
    /// -> `ClientDisconnect` QC.
    pub fn ClInput_Glue_Disconnect() -> c_int;

    /* Engine C symbols cl_input.c calls directly; none of these can raise. */

    /// `cl_main.c` -- johnfitz's variable pitch clamps, and `lookspring`,
    /// read by `IN_MLookUp`. All three are registered by `cl_main.c`, which
    /// is still C in T7.2.
    pub static mut cl_maxpitch: crate::cvar_t;
    pub static mut cl_minpitch: crate::cvar_t;
    pub static mut lookspring: crate::cvar_t;

    /// `Quake/view.h`. Neither touches the VM nor the network, so neither can
    /// `Host_Error`.
    pub fn V_StartPitchDrift();
    pub fn V_StopPitchDrift();
}

// `NET_SendUnreliableMessage` and `NET_QSocketGetProQuakeAngleHack` are
// already declared by `crate::sv_send` / `crate::sv_user`; re-declaring them
// here with a different pointer type would trip `clashing_extern_declarations`,
// so `cl_input.c`'s two call sites reuse those.
