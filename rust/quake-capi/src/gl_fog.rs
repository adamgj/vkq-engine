//! `Quake/gl_fog.c` -- global fog state, fades and the per-frame fog push
//! constants (Rust migration Phase 8 M7, Pattern A whole-file swap, ADR-015).
//!
//! `Quake/gl_fog_glue.c` keeps the `%g`-formatting `Fog_GetFogCommand`
//! (ADR-005 gap), the `fog` console command (`Con_Printf ("%f")` usage text),
//! `Fog_Init` and the unregistered `r_vfog` cvar. The four `fog_*` globals
//! and `fade_done` are exported with their C names because the glue reads
//! them.

use core::ffi::{c_char, c_float, c_int};
use core::ptr;

use ash::vk;
use quake_c_sys as c;
use quake_render::cb::{self, CmdProcs};
use quake_types::host::ClientState;
use quake_types::render::CbContext;

extern "C" {
    /// `client_state_t cl` (ADR-007 row closed in Phase 7; storage in
    /// `crate::cl_main` under `use_rust_host`, else `cl_main.c`).
    static mut cl: ClientState;
}
use crate::gl_rmisc::{vg, with_ctx};

const DEFAULT_DENSITY: c_float = 0.0;
const DEFAULT_GRAY: c_float = 0.3;

// `gl_fog.c:33-37` -- the global fog state (not `static` in C).
#[no_mangle]
pub static mut fog_density: c_float = 0.0;
#[no_mangle]
pub static mut fog_red: c_float = 0.0;
#[no_mangle]
pub static mut fog_green: c_float = 0.0;
#[no_mangle]
pub static mut fog_blue: c_float = 0.0;
#[no_mangle]
pub static mut old_density: c_float = 0.0;
#[no_mangle]
pub static mut old_red: c_float = 0.0;
#[no_mangle]
pub static mut old_green: c_float = 0.0;
#[no_mangle]
pub static mut old_blue: c_float = 0.0;
#[no_mangle]
pub static mut fade_time: c_float = 0.0;
#[no_mangle]
pub static mut fade_done: c_float = 0.0;

#[inline]
unsafe fn cl_time() -> f64 {
    // SAFETY: `cl` is the Rust-owned client state; only `time` is read.
    unsafe { (*ptr::addr_of!(cl)).time }
}

/// `Q_rint` (`q_minmax.h`) over a `float`, keeping C's `+ 0.5` promotion.
fn q_rint(x: c_float) -> c_int {
    if x > 0.0 {
        (f64::from(x) + 0.5) as c_int
    } else {
        (f64::from(x) - 0.5) as c_int
    }
}

/// `Fog_Update` -- update internal variables.
///
/// # Safety
/// Main thread only: the fog state and `cl` are unsynchronised globals.
#[no_mangle]
pub unsafe extern "C" fn Fog_Update(
    density: c_float,
    red: c_float,
    green: c_float,
    blue: c_float,
    time: c_float,
) {
    // SAFETY: the fog globals are main-thread state (server-message parsing,
    // the `fog` command and map loads all run there).
    unsafe {
        if time > 0.0 {
            // check for a fade in progress
            let now = cl_time();
            if f64::from(fade_done) > now && fade_time != 0.0 {
                let f = ((f64::from(fade_done) - now) / f64::from(fade_time)) as c_float;
                old_density = (f64::from(f * old_density)
                    + (1.0 - f64::from(f)) * f64::from(fog_density))
                    as c_float;
                old_red =
                    (f64::from(f * old_red) + (1.0 - f64::from(f)) * f64::from(fog_red)) as c_float;
                old_green = (f64::from(f * old_green) + (1.0 - f64::from(f)) * f64::from(fog_green))
                    as c_float;
                old_blue = (f64::from(f * old_blue) + (1.0 - f64::from(f)) * f64::from(fog_blue))
                    as c_float;
            } else {
                old_density = fog_density;
                old_red = fog_red;
                old_green = fog_green;
                old_blue = fog_blue;
            }
        }

        fog_density = density;
        fog_red = red;
        fog_green = green;
        fog_blue = blue;
        fade_time = time;
        fade_done = (cl_time() + f64::from(time)) as c_float;
    }
}

/// `Fog_ParseServerMessage` -- handle an `svc_fog` message from the server.
///
/// # Safety
/// Main thread, while `net_message` holds the `svc_fog` payload.
#[no_mangle]
pub unsafe extern "C" fn Fog_ParseServerMessage() {
    // SAFETY: the `MSG_Read*` readers run on the main thread over `net_message`.
    unsafe {
        let density = (f64::from(c::sv_user::MSG_ReadByte()) / 255.0) as c_float;
        let red = (f64::from(c::sv_user::MSG_ReadByte()) / 255.0) as c_float;
        let green = (f64::from(c::sv_user::MSG_ReadByte()) / 255.0) as c_float;
        let blue = (f64::from(c::sv_user::MSG_ReadByte()) / 255.0) as c_float;
        let time = f64::max(0.0, f64::from(c::sv_user::MSG_ReadShort()) / 100.0) as c_float;
        Fog_Update(density, red, green, blue, time);
    }
}

/// `Fog_ParseWorldspawn` -- called at map load.
unsafe fn parse_worldspawn() {
    const CPE_ALLOWTRUNC: c_int = 1;
    let mut key = [0 as c_char; 128];
    let mut value = [0 as c_char; 4096];

    // SAFETY: main-thread map load; `cl.worldmodel->entities` is the loaded
    // entity lump (NUL-terminated); `COM_ThreadToken` is this thread's
    // `com_token`, valid until the next parse.
    unsafe {
        // initially no fog
        fog_density = DEFAULT_DENSITY;
        fog_red = DEFAULT_GRAY;
        fog_green = DEFAULT_GRAY;
        fog_blue = DEFAULT_GRAY;

        old_density = DEFAULT_DENSITY;
        old_red = DEFAULT_GRAY;
        old_green = DEFAULT_GRAY;
        old_blue = DEFAULT_GRAY;

        fade_time = 0.0;
        fade_done = 0.0;

        let mut data = c::COM_Parse((*(*ptr::addr_of!(cl)).worldmodel).entities);
        if data.is_null() {
            return; // error
        }
        if *c::COM_ThreadToken() != b'{' as c_char {
            return; // error
        }
        loop {
            data = c::COM_Parse(data);
            if data.is_null() {
                return; // error
            }
            let token = c::COM_ThreadToken();
            if *token == b'}' as c_char {
                break; // end of worldspawn
            }
            if *token == b'_' as c_char {
                c::cl_main::q_strlcpy(key.as_mut_ptr(), token.add(1), key.len());
            } else {
                c::cl_main::q_strlcpy(key.as_mut_ptr(), token, key.len());
            }
            // remove trailing spaces
            let mut len = c::menu::strlen(key.as_ptr());
            while len > 0 && key[len - 1] == b' ' as c_char {
                key[len - 1] = 0;
                len -= 1;
            }
            data = c::progs_edict_dispatch::COM_ParseEx(data, CPE_ALLOWTRUNC);
            if data.is_null() {
                return; // error
            }
            c::cl_main::q_strlcpy(value.as_mut_ptr(), c::COM_ThreadToken(), value.len());

            if c::cl_main::strcmp(c"fog".as_ptr(), key.as_ptr()) == 0 {
                c::cl_demo::sscanf(
                    value.as_ptr(),
                    c"%f %f %f %f".as_ptr(),
                    ptr::addr_of_mut!(fog_density),
                    ptr::addr_of_mut!(fog_red),
                    ptr::addr_of_mut!(fog_green),
                    ptr::addr_of_mut!(fog_blue),
                );
            }
        }
    }
}

/// `Fog_GetColor` -- calculates fog color for this frame, taking into account
/// fade times.
///
/// # Safety
/// `c` points at four writable floats; fog state is read on the main thread
/// or after `Fog_SetupFrame` has published it for the frame.
#[no_mangle]
pub unsafe extern "C" fn Fog_GetColor(c: *mut c_float) {
    // SAFETY: `c` points at four floats; the fog globals are read on the
    // main thread (the render tasks that fog run after `Fog_SetupFrame`).
    unsafe {
        let c = core::slice::from_raw_parts_mut(c, 4);
        let now = cl_time();
        if f64::from(fade_done) > now && fade_time != 0.0 {
            let f = ((f64::from(fade_done) - now) / f64::from(fade_time)) as c_float;
            c[0] = (f64::from(f * old_red) + (1.0 - f64::from(f)) * f64::from(fog_red)) as c_float;
            c[1] =
                (f64::from(f * old_green) + (1.0 - f64::from(f)) * f64::from(fog_green)) as c_float;
            c[2] =
                (f64::from(f * old_blue) + (1.0 - f64::from(f)) * f64::from(fog_blue)) as c_float;
            c[3] = 1.0;
        } else {
            c[0] = fog_red;
            c[1] = fog_green;
            c[2] = fog_blue;
            c[3] = 1.0;
        }

        // find closest 24-bit RGB value, so solid-colored sky can match the fog perfectly
        for v in &mut c[..3] {
            *v = (q_rint(*v * 255.0) as c_float) / 255.0;
        }
    }
}

/// `Fog_GetDensity` -- returns current density of fog.
///
/// # Safety
/// As `Fog_GetColor`.
#[no_mangle]
pub unsafe extern "C" fn Fog_GetDensity() -> c_float {
    // SAFETY: as `Fog_GetColor`.
    unsafe {
        let now = cl_time();
        if f64::from(fade_done) > now && fade_time != 0.0 {
            let f = ((f64::from(fade_done) - now) / f64::from(fade_time)) as c_float;
            (f64::from(f * old_density) + (1.0 - f64::from(f)) * f64::from(fog_density)) as c_float
        } else {
            fog_density
        }
    }
}

/// `Fog_ResetFade` -- called when client time may jump.
///
/// # Safety
/// Main thread only: the fog state and `cl` are unsynchronised globals.
#[no_mangle]
pub unsafe extern "C" fn Fog_ResetFade() {
    // SAFETY: main-thread fog state.
    unsafe { fade_done = 0.0 }
}

/// The `fog_values` push constant: the clamped fog colour and density / 64.
unsafe fn fog_values() -> [c_float; 4] {
    let mut fog_color = [0.0 as c_float; 4];
    // SAFETY: `fog_color` has four floats.
    unsafe { Fog_GetColor(fog_color.as_mut_ptr()) };
    [
        fog_color[0].clamp(0.0, 1.0),
        fog_color[1].clamp(0.0, 1.0),
        fog_color[2].clamp(0.0, 1.0),
        // SAFETY: as above.
        unsafe { Fog_GetDensity() } / 64.0,
    ]
}

fn push_fog_values(procs: &CmdProcs, cbx: &CbContext, values: &[c_float; 4]) {
    let bytes: [u8; 16] = bytemuck_floats(values);
    cb::push_constants(
        procs,
        cbx,
        vk::ShaderStageFlags::ALL_GRAPHICS,
        16 * 4,
        &bytes,
    );
}

fn bytemuck_floats(values: &[c_float; 4]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, v) in values.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_ne_bytes());
    }
    out
}

/// `Fog_SetupFrame` -- called at the beginning of each frame.
///
/// # Safety
/// `cbx` is a live recording context, unaliased for the call; main thread.
#[no_mangle]
pub unsafe extern "C" fn Fog_SetupFrame(cbx: *mut CbContext) {
    // SAFETY: `cbx` is the recording primary context (ADR-011 mirror) and no
    // other reference to it exists for the call.
    let cbx = unsafe { &mut *cbx };
    // SAFETY: fog state is read on the main thread.
    let values = unsafe { fog_values() };
    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        let variant = cb::main_pass_pipeline_variant(cbx.render_pass_index);
        let pipeline = vg!(ctx, world_pipelines)[variant][0];
        cb::bind_pipeline(&procs, cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
        push_fog_values(&procs, cbx, &values);
    });
}

/// `Fog_EnableGFog` -- called before drawing stuff that should be fogged.
///
/// # Safety
/// `cbx` is a live recording context, unaliased for the call; main thread.
#[no_mangle]
pub unsafe extern "C" fn Fog_EnableGFog(cbx: *mut CbContext) {
    // SAFETY: `cbx` is a recording context (ADR-011 mirror), unaliased for
    // the call; fog state is read on the main thread.
    let cbx = unsafe { &*cbx };
    // SAFETY: fog state is read on the main thread.
    let values = unsafe { fog_values() };
    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        push_fog_values(&procs, cbx, &values);
    });
}

/// `Fog_DisableGFog` -- called after drawing stuff that should be fogged.
///
/// # Safety
/// `cbx` is a live recording context, unaliased for the call; main thread.
#[no_mangle]
pub unsafe extern "C" fn Fog_DisableGFog(cbx: *mut CbContext) {
    // SAFETY: as `Fog_EnableGFog`.
    let cbx = unsafe { &*cbx };
    with_ctx(|ctx| {
        debug_assert!(cbx.current_pipeline.layout.handle == vg!(ctx, basic_pipeline_layout).handle);
        let procs = CmdProcs::new(ctx.vg);
        push_fog_values(&procs, cbx, &[0.0; 4]);
    });
}

/// `Fog_DrawVFog` (volumetric fog; empty in C).
#[no_mangle]
pub extern "C" fn Fog_DrawVFog() {}

/// `Fog_MarkModels` (volumetric fog; empty in C).
#[no_mangle]
pub extern "C" fn Fog_MarkModels() {}

/// `Fog_NewMap` -- called whenever a map is loaded.
///
/// # Safety
/// Main thread only: the fog state and `cl` are unsynchronised globals.
#[no_mangle]
pub unsafe extern "C" fn Fog_NewMap() {
    // SAFETY: main-thread map load.
    unsafe { parse_worldspawn() }; // for global fog
    Fog_MarkModels(); // for volumetric fog
}
