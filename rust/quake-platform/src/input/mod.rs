//! `Quake/in_sdl.c` + `in_sdl.h` (Rust migration Phase 9 M3, ADR-017): the
//! backend-neutral input layer -- mouse accumulation, joystick axis math,
//! gamepad key emulation, text-input mode, the `IN_*` entry points and the
//! event dispatch -- over one of two SDL backends (`sdl2.rs`/`sdl3.rs`,
//! selected by the crate feature) that own the SDL calls, the gamepad
//! handle and the mouse-event filter.
//!
//! State is the same set of file-scope statics `in_sdl.c` had, touched on
//! the main thread only (the SDL event filter runs on the thread that pushes
//! events, which for the mouse-motion path is the main thread's
//! `SDL_PollEvent`). Cvars stay C-owned in `Quake/in_sdl_glue.c` (ADR-007).
//!
//! Callees that can raise (`Key_Event`, `Key_EventWithKeycode`,
//! `Char_Event`, `CL_Disconnect`, `Sys_Quit`, the `scr_conscale` callback,
//! `Cvar_RegisterVariable`) are reached through `Host_Guard` trampolines in
//! the glue and their status is propagated with `raise!` (ADR-009 rule 3);
//! `init`, `send_key_events` and `commands` therefore return a status their
//! C wrappers `Host_Reraise`. `IN_Move` reaches nothing that raises, so it
//! stays `void`.

use core::ffi::{c_char, c_int, CStr};
use core::ptr::{addr_of, addr_of_mut};

use quake_c_sys as sys;
use quake_c_sys::cl_input::{
    cl_alwaysrun, cl_forwardspeed, cl_maxpitch, cl_minpitch, cl_movespeedkey, in_mlook, in_speed,
    in_strafe, V_StopPitchDrift,
};
use quake_c_sys::cl_main::{lookstrafe, m_forward, m_pitch, m_side, m_yaw, sensitivity};
use quake_c_sys::cl_parse::vid;
use quake_c_sys::host::host_parms;
use quake_c_sys::input::*;
use quake_c_sys::keys::key_dest;
use quake_c_sys::libm;
use quake_c_sys::menu::scr_fov;
use quake_c_sys::sv_user::sv_maxspeed;
use quake_c_sys::view::{noclip_anglehack, CL_AngleLocked};
use quake_types::host::{ClientState, ClientStatic, UserCmd, CA_CONNECTED};
use quake_types::refdef::RefDef;

pub mod keys;
pub mod scancode;

#[cfg(feature = "sdl2")]
#[path = "sdl2.rs"]
mod backend;
#[cfg(feature = "sdl3")]
#[path = "sdl3.rs"]
mod backend;

use keys::*;

/// A `Host_Guard` status: 0 is `HOST_GUARD_OK`.
pub type Raise = c_int;

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
    /// ADR-007 rows closed in Phase 7; storage in `quake-capi::cl_main`.
    static mut cl: ClientState;
    static mut cls: ClientStatic;
    /// `Quake/gl_rmain.c` (C-owned) -- `refdef_t r_refdef`.
    static mut r_refdef: RefDef;
}

/// `client.h:68`.
const SIGNONS: c_int = 4;
/// `keys.h` `keydest_t`.
const KEY_GAME: c_int = 0;
const KEY_CONSOLE: c_int = 1;
/// `mathlib.h` angle indices.
const PITCH: usize = 0;
const YAW: usize = 1;

/// `SDL_GAMEPAD_AXIS_*` / `SDL_CONTROLLER_AXIS_*` (identical numbering).
const AXIS_LEFTX: usize = 0;
const AXIS_LEFTY: usize = 1;
const AXIS_RIGHTX: usize = 2;
const AXIS_RIGHTY: usize = 3;
const AXIS_LEFT_TRIGGER: usize = 4;
const AXIS_RIGHT_TRIGGER: usize = 5;
const AXIS_COUNT: usize = 6;
/// `SDL_GAMEPAD_BUTTON_COUNT` (26) / `SDL_CONTROLLER_BUTTON_MAX` (21).
const BUTTON_COUNT: usize = backend::BUTTON_COUNT;

/// in_sdl.h `buttonremap[]`: SDL mouse buttons 1..5 to Quake keys.
const BUTTONREMAP: [c_int; 5] = [K_MOUSE1, K_MOUSE3, K_MOUSE2, K_MOUSE4, K_MOUSE5];

// SAFETY invariant for every static below: main-thread only, like the
// in_sdl.c statics they replace; nothing holds a reference across a call
// back into the engine (IN_Commands can re-enter through SCR_ModalMessage).
static mut TEXTMODE: bool = false;
static mut NO_MOUSE: bool = false;
static mut TOTAL_DX: f32 = 0.0;
static mut TOTAL_DY: f32 = 0.0;
static mut JOY_BUTTONSTATE: [bool; BUTTON_COUNT] = [false; BUTTON_COUNT];
static mut JOY_AXISSTATE: [f32; AXIS_COUNT] = [0.0; AXIS_COUNT];
static mut JOY_BUTTONTIMER: [f64; BUTTON_COUNT] = [0.0; BUTTON_COUNT];
static mut JOY_EMULATEDKEYTIMER: [f64; 6] = [0.0; 6];

/// One SDL event after the backend has read the union; `TextInput` borrows
/// the event's own bytes.
pub(crate) enum Event<'a> {
    FocusGained,
    FocusLost,
    SizeChanged(c_int, c_int),
    TextInput(&'a CStr),
    Key {
        down: bool,
        scancode: c_int,
        keycode: c_int,
    },
    MouseButton {
        button: c_int,
        down: bool,
        x: f32,
        y: f32,
    },
    MouseWheel(f32),
    MouseMotion(f32, f32),
    Quit,
}

/* ---------------------------------------------------------------------------
 * Mouse activation (in_sdl.c IN_Activate .. IN_HideCursor).
 */

/// in_sdl.c `IN_Activate`.
///
/// # Safety
/// Main thread, after `VID_Init` (mirrors the C entry points' contract).
pub unsafe fn activate() {
    // SAFETY: main thread; see the static invariant
    unsafe {
        if NO_MOUSE {
            return;
        }
        #[cfg(target_os = "macos")]
        backend::warp_mouse_to_center();
        if !backend::set_relative_mouse_mode(true) {
            sys::Con_Printf(backend::RELATIVE_MOUSE_WARNING.as_ptr());
        }
        backend::end_ignoring_mouse_events();
        TOTAL_DX = 0.0;
        TOTAL_DY = 0.0;
    }
}

/// in_sdl.c `IN_Deactivate`.
///
/// # Safety
/// Main thread, after `VID_Init` (mirrors the C entry points' contract).
pub unsafe fn deactivate(free_cursor: bool) {
    // SAFETY: main thread; see the static invariant
    unsafe {
        if NO_MOUSE {
            return;
        }
        if free_cursor {
            backend::set_relative_mouse_mode(false);
        }
        backend::begin_ignoring_mouse_events();
    }
}

/// in_sdl.c `IN_DeactivateForConsole`.
///
/// # Safety
/// Main thread, after `VID_Init` (mirrors the C entry points' contract).
pub unsafe fn deactivate_for_console() {
    // SAFETY: as `deactivate`
    unsafe { deactivate(true) }
}

/// in_sdl.c `IN_ScaleMouseCoords`: window pixels to `vid` pixels.
///
/// # Safety
/// Main thread, after `VID_Init` (mirrors the C entry points' contract).
pub unsafe fn scale_mouse_coords(mut x: f32, mut y: f32) -> (c_int, c_int) {
    // SAFETY: `vid` is the engine's live viddef_t; main thread
    unsafe {
        let (w, h) = backend::window_size();
        if w > 0 && h > 0 {
            x = x * vid.width as f32 / w as f32;
            y = y * vid.height as f32 / h as f32;
        }
    }
    (x as c_int, y as c_int)
}

/// in_sdl.c `IN_GetMousePos`.
///
/// # Safety
/// Main thread, after `VID_Init` (mirrors the C entry points' contract).
pub unsafe fn get_mouse_pos() -> (c_int, c_int) {
    // SAFETY: main thread
    unsafe {
        let (x, y) = backend::mouse_position();
        scale_mouse_coords(x, y)
    }
}

/// in_sdl.c `IN_HideCursor`.
///
/// # Safety
/// Main thread, after `VID_Init` (mirrors the C entry points' contract).
pub unsafe fn hide_cursor() {
    // SAFETY: main thread; see the static invariant
    unsafe {
        if NO_MOUSE {
            return;
        }
        if !backend::set_relative_mouse_mode(true) {
            sys::Con_Printf(backend::RELATIVE_MOUSE_WARNING.as_ptr());
        }
    }
}

/// The mouse-motion half of in_sdl.c `IN_FilterMouseEvents`, called from the
/// backend's SDL event filter.
///
/// # Safety
/// Main thread, from inside the SDL event filter (SDL calls it on the
/// thread that pushes the event; the engine pumps on the main thread only).
pub(crate) unsafe fn filter_mouse_motion(x: f32, y: f32) {
    // SAFETY: `key_dest` is the engine's keydest_t; Con_Mousemove cannot raise
    unsafe {
        if key_dest == KEY_CONSOLE {
            let (cx, cy) = scale_mouse_coords(x, y);
            Con_Mousemove(cx, cy);
        }
    }
}

/* ---------------------------------------------------------------------------
 * Init / shutdown.
 */

/// in_sdl.c `IN_Init`; the `Cvar_RegisterVariable` calls can raise.
///
/// # Safety
/// Main thread, after `VID_Init` (mirrors the C entry points' contract).
pub unsafe fn init() -> Raise {
    // SAFETY: main thread, before the first frame; the cvar objects are
    // in_sdl_glue.c's statics, live for the whole run
    unsafe {
        TEXTMODE = Key_TextEntry();
        if TEXTMODE {
            backend::start_text_input();
        } else {
            backend::stop_text_input();
        }

        if sys::safemode != 0 || sys::COM_CheckParm(c"-nomouse".as_ptr()) != 0 {
            NO_MOUSE = true;
            /* discard all mouse events when input is deactivated */
            backend::begin_ignoring_mouse_events();
        }

        raise!(InSdl_Glue_RegisterVariable(addr_of_mut!(in_debugkeys)));
        raise!(InSdl_Glue_RegisterVariable(addr_of_mut!(
            joy_sensitivity_yaw
        )));
        raise!(InSdl_Glue_RegisterVariable(addr_of_mut!(
            joy_sensitivity_pitch
        )));
        raise!(InSdl_Glue_RegisterVariable(addr_of_mut!(joy_deadzone_look)));
        raise!(InSdl_Glue_RegisterVariable(addr_of_mut!(joy_deadzone_move)));
        raise!(InSdl_Glue_RegisterVariable(addr_of_mut!(
            joy_outer_threshold_look
        )));
        raise!(InSdl_Glue_RegisterVariable(addr_of_mut!(
            joy_outer_threshold_move
        )));
        raise!(InSdl_Glue_RegisterVariable(addr_of_mut!(
            joy_deadzone_trigger
        )));
        raise!(InSdl_Glue_RegisterVariable(addr_of_mut!(joy_invert)));
        raise!(InSdl_Glue_RegisterVariable(addr_of_mut!(joy_exponent)));
        raise!(InSdl_Glue_RegisterVariable(addr_of_mut!(joy_exponent_move)));
        raise!(InSdl_Glue_RegisterVariable(addr_of_mut!(joy_swapmovelook)));
        raise!(InSdl_Glue_RegisterVariable(addr_of_mut!(joy_enable)));

        activate();
        startup_joystick();
    }
    0
}

/// in_sdl.c `IN_Shutdown`.
///
/// # Safety
/// Main thread, after `VID_Init` (mirrors the C entry points' contract).
pub unsafe fn shutdown() {
    // SAFETY: main thread
    unsafe {
        deactivate(true);
        backend::shutdown_joystick();
    }
}

/// in_sdl3.c / in_sdl2.c `IN_StartupJoystick`: the shared part (`-nojoy`,
/// subsystem init, `gamecontrollerdb.txt` from basedir then userdir); the
/// enumerate-and-open pass differs per SDL generation and lives in the
/// backend.
unsafe fn startup_joystick() {
    // SAFETY: main thread; host_parms points at main's quakeparms_t for the
    // whole run and com_basedir is a NUL-terminated char array
    unsafe {
        if sys::COM_CheckParm(c"-nojoy".as_ptr()) != 0 {
            return;
        }
        if !backend::init_gamepad_subsystem() {
            sys::Con_Warning(backend::GAMEPAD_INIT_WARNING.as_ptr());
            return;
        }

        let basedir = addr_of!(sys::manual::com_basedir).cast::<c_char>();
        load_mappings(basedir);
        let parms = host_parms;
        if (*parms).userdir != (*parms).basedir {
            load_mappings((*parms).userdir);
        }

        backend::open_first_gamepad();
    }
}

/// `"%s/gamecontrollerdb.txt"` under `dir` (MAX_OSPATH-bounded like the
/// `q_snprintf` in C), loaded and reported.
unsafe fn load_mappings(dir: *const c_char) {
    use sys::MAX_OSPATH; // PATH_MAX per platform, as q_snprintf's buffer in C
                         // SAFETY: `dir` is a NUL-terminated engine path
    unsafe {
        let dir = CStr::from_ptr(dir).to_bytes();
        let mut path = [0u8; MAX_OSPATH];
        for (slot, &b) in path[..MAX_OSPATH - 1]
            .iter_mut()
            .zip(dir.iter().chain(b"/gamecontrollerdb.txt"))
        {
            *slot = b;
        }
        let nmappings = backend::add_gamepad_mappings_from_file(path.as_ptr().cast());
        if nmappings > 0 {
            sys::Con_Printf(
                c"%d mappings loaded from gamecontrollerdb.txt\n".as_ptr(),
                nmappings,
            );
        }
    }
}

/* ---------------------------------------------------------------------------
 * Mouse motion.
 */

/// in_sdl.c `IN_MouseMotion`.
///
/// # Safety
/// Main thread, after `VID_Init` (mirrors the C entry points' contract).
pub unsafe fn mouse_motion(dx: f32, dy: f32) {
    // SAFETY: main thread; cls/cl are the live client structs
    unsafe {
        if cls.state != CA_CONNECTED
            || cls.signon != SIGNONS
            || key_dest != KEY_GAME
            || CL_AngleLocked()
        {
            TOTAL_DX = 0.0;
            TOTAL_DY = 0.0;
            return;
        }
        TOTAL_DX += dx;
        TOTAL_DY += dy;
    }
}

/* ---------------------------------------------------------------------------
 * Joystick axis math (in_sdl.c IN_AxisMagnitude .. IN_KeyForControllerButton).
 */

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct JoyAxis {
    pub x: f32,
    pub y: f32,
}

/// in_sdl.c `IN_AxisMagnitude`.
pub fn axis_magnitude(axis: JoyAxis) -> f32 {
    libm::sqrtf(axis.x * axis.x + axis.y * axis.y)
}

/// in_sdl.c `IN_ApplyEasing`: raise the magnitude to `exponent`, keeping
/// signs; assumes the magnitude has been clamped at 1.
pub fn apply_easing(axis: JoyAxis, exponent: f32) -> JoyAxis {
    let magnitude = axis_magnitude(axis);
    if magnitude == 0.0 {
        return JoyAxis::default();
    }
    let eased_magnitude = libm::powf(magnitude, exponent);
    JoyAxis {
        x: axis.x * (eased_magnitude / magnitude),
        y: axis.y * (eased_magnitude / magnitude),
    }
}

/// in_sdl.c `IN_ApplyDeadzone`: circular inner deadzone and outer threshold,
/// magnitude clamped at 1. The `q_min (1.0, ...)` is evaluated in double as
/// the C macro does.
pub fn apply_deadzone(axis: JoyAxis, deadzone: f32, outer_threshold: f32) -> JoyAxis {
    let magnitude = axis_magnitude(axis);
    if magnitude > deadzone {
        let new_magnitude = f64::min(
            1.0,
            f64::from(magnitude - deadzone)
                / (1.0 - f64::from(deadzone) - f64::from(outer_threshold)),
        ) as f32;
        let scale = new_magnitude / magnitude;
        JoyAxis {
            x: axis.x * scale,
            y: axis.y * scale,
        }
    } else {
        JoyAxis::default()
    }
}

/// in_sdl.c `IN_KeyForControllerButton`, keyed on the button index (the
/// SDL2 `SDL_CONTROLLER_BUTTON_*` and SDL3 `SDL_GAMEPAD_BUTTON_*` values
/// agree for 0..=20).
pub fn key_for_controller_button(button: usize) -> c_int {
    match button {
        0 => K_ABUTTON,     // SOUTH / A
        1 => K_BBUTTON,     // EAST / B
        2 => K_XBUTTON,     // WEST / X
        3 => K_YBUTTON,     // NORTH / Y
        4 => K_TAB,         // BACK
        6 => K_ESCAPE,      // START
        7 => K_LTHUMB,      // LEFT_STICK
        8 => K_RTHUMB,      // RIGHT_STICK
        9 => K_LSHOULDER,   // LEFT_SHOULDER
        10 => K_RSHOULDER,  // RIGHT_SHOULDER
        11 => K_UPARROW,    // DPAD_UP
        12 => K_DOWNARROW,  // DPAD_DOWN
        13 => K_LEFTARROW,  // DPAD_LEFT
        14 => K_RIGHTARROW, // DPAD_RIGHT
        15 => K_MISC1,      // MISC1
        16 => K_PADDLE1,    // RIGHT_PADDLE1 / PADDLE1
        17 => K_PADDLE2,    // LEFT_PADDLE1 / PADDLE2
        18 => K_PADDLE3,    // RIGHT_PADDLE2 / PADDLE3
        19 => K_PADDLE4,    // LEFT_PADDLE2 / PADDLE4
        20 => K_TOUCHPAD,   // TOUCHPAD
        _ => 0,             // GUIDE (5), MISC2..6, anything newer
    }
}

/// in_sdl.c `IN_JoyKeyEvent`: press/release transitions plus key repeats
/// while held (DarkPlaces). `Key_Event` can raise.
unsafe fn joy_key_event(wasdown: bool, isdown: bool, key: c_int, timer: *mut f64) -> Raise {
    // SAFETY: `timer` points into one of the timer statics; nothing else
    // holds it across the Key_Event call
    unsafe {
        // we can't use `realtime` for key repeats because it is not monotonic
        let currenttime = sys::Sys_DoubleTime();
        if wasdown {
            if isdown {
                if currenttime >= *timer {
                    *timer = currenttime + 0.1;
                    raise!(InSdl_Glue_KeyEvent(key, true));
                }
            } else {
                *timer = 0.0;
                raise!(InSdl_Glue_KeyEvent(key, false));
            }
        } else if isdown {
            *timer = currenttime + 0.5;
            raise!(InSdl_Glue_KeyEvent(key, true));
        }
    }
    0
}

/// in_sdl.c `IN_Commands`: key events for gamepad buttons plus the emulated
/// menu arrows and trigger keys.
///
/// # Safety
/// Main thread, after `VID_Init` (mirrors the C entry points' contract).
pub unsafe fn commands() -> Raise {
    // SAFETY: main thread; array accesses go through raw pointers so a
    // reentrant IN_Commands (SCR_ModalMessage from Key_Event) sees no
    // outstanding borrow
    unsafe {
        let stickthreshold: f32 = 0.9;
        let triggerthreshold: f32 = joy_deadzone_trigger.value;

        if joy_enable.value == 0.0 {
            return 0;
        }
        if !backend::joystick_active() {
            return 0;
        }

        // emit key events for controller buttons
        for i in 0..BUTTON_COUNT {
            let newstate = backend::gamepad_button(i);
            let slot = addr_of_mut!(JOY_BUTTONSTATE).cast::<bool>().add(i);
            let oldstate = *slot;
            *slot = newstate;
            // NOTE: This can cause a reentrant call of IN_Commands, via
            // SCR_ModalMessage when confirming a new game.
            raise!(joy_key_event(
                oldstate,
                newstate,
                key_for_controller_button(i),
                addr_of_mut!(JOY_BUTTONTIMER).cast::<f64>().add(i),
            ));
        }

        let mut newaxisstate = [0.0f32; AXIS_COUNT];
        for (i, v) in newaxisstate.iter_mut().enumerate() {
            *v = f32::from(backend::gamepad_axis(i)) / 32768.0;
        }
        // Re-read per call like C: a reentrant `IN_Commands` (SCR_ModalMessage
        // from Key_Event) may store into `joy_axisstate` between two calls.
        let old = |n: usize| (*addr_of!(JOY_AXISSTATE))[n];
        let timer = |n: usize| addr_of_mut!(JOY_EMULATEDKEYTIMER).cast::<f64>().add(n);

        // emit emulated arrow keys so the analog sticks can be used in the menu
        if key_dest != KEY_GAME {
            raise!(joy_key_event(
                old(AXIS_LEFTX) < -stickthreshold,
                newaxisstate[AXIS_LEFTX] < -stickthreshold,
                K_LEFTARROW,
                timer(0),
            ));
            raise!(joy_key_event(
                old(AXIS_LEFTX) > stickthreshold,
                newaxisstate[AXIS_LEFTX] > stickthreshold,
                K_RIGHTARROW,
                timer(1),
            ));
            raise!(joy_key_event(
                old(AXIS_LEFTY) < -stickthreshold,
                newaxisstate[AXIS_LEFTY] < -stickthreshold,
                K_UPARROW,
                timer(2),
            ));
            raise!(joy_key_event(
                old(AXIS_LEFTY) > stickthreshold,
                newaxisstate[AXIS_LEFTY] > stickthreshold,
                K_DOWNARROW,
                timer(3),
            ));
        }

        // emit emulated keys for the analog triggers
        raise!(joy_key_event(
            old(AXIS_LEFT_TRIGGER) > triggerthreshold,
            newaxisstate[AXIS_LEFT_TRIGGER] > triggerthreshold,
            K_LTRIGGER,
            timer(4),
        ));
        raise!(joy_key_event(
            old(AXIS_RIGHT_TRIGGER) > triggerthreshold,
            newaxisstate[AXIS_RIGHT_TRIGGER] > triggerthreshold,
            K_RTRIGGER,
            timer(5),
        ));

        *addr_of_mut!(JOY_AXISSTATE) = newaxisstate;
    }
    0
}

/// in_sdl.c `IN_JoyMove`.
unsafe fn joy_move(cmd: &mut UserCmd) {
    // SAFETY: main thread; cvars are in_sdl_glue.c/cl_input_glue.c statics
    unsafe {
        if joy_enable.value == 0.0 {
            return;
        }
        if !backend::joystick_active() {
            return;
        }
        if cl.paused || key_dest != KEY_GAME {
            return;
        }

        let axes = *addr_of_mut!(JOY_AXISSTATE);
        let mut move_raw = JoyAxis {
            x: axes[AXIS_LEFTX],
            y: axes[AXIS_LEFTY],
        };
        let mut look_raw = JoyAxis {
            x: axes[AXIS_RIGHTX],
            y: axes[AXIS_RIGHTY],
        };
        if joy_swapmovelook.value != 0.0 {
            core::mem::swap(&mut move_raw, &mut look_raw);
        }

        let move_deadzone = apply_deadzone(
            move_raw,
            joy_deadzone_move.value,
            joy_outer_threshold_move.value,
        );
        let look_deadzone = apply_deadzone(
            look_raw,
            joy_deadzone_look.value,
            joy_outer_threshold_look.value,
        );
        let move_eased = apply_easing(move_deadzone, joy_exponent_move.value);
        let look_eased = apply_easing(look_deadzone, joy_exponent.value);

        let speed: f32 = if ((in_speed.state & 1) != 0)
            ^ (cl_alwaysrun.value != 0.0 || cl_forwardspeed.value >= sv_maxspeed.value)
        {
            // running
            sv_maxspeed.value
        } else if cl_forwardspeed.value >= sv_maxspeed.value {
            // not running, with always run = vanilla
            f32::min(
                sv_maxspeed.value,
                cl_forwardspeed.value / cl_movespeedkey.value,
            )
        } else {
            // not running, with always run = off or quakespasm
            cl_forwardspeed.value
        };

        cmd.sidemove += speed * move_eased.x;
        cmd.forwardmove -= speed * move_eased.y;

        if CL_AngleLocked() {
            return;
        }

        let frametime = sys::host_frametime;
        // The float product is promoted to double, the compound assignment
        // runs in double and rounds to float once (in_sdl.c:569-570).
        cl.viewangles[YAW] = (f64::from(cl.viewangles[YAW])
            - f64::from(look_eased.x * joy_sensitivity_yaw.value) * frametime)
            as f32;
        let invert: f64 = if joy_invert.value != 0.0 { -1.0 } else { 1.0 };
        cl.viewangles[PITCH] = (f64::from(cl.viewangles[PITCH])
            + f64::from(look_eased.y * joy_sensitivity_pitch.value) * invert * frametime)
            as f32;

        if look_eased.x != 0.0 || look_eased.y != 0.0 {
            V_StopPitchDrift();
        }

        /* johnfitz -- variable pitch clamping */
        if cl.viewangles[PITCH] > cl_maxpitch.value {
            cl.viewangles[PITCH] = cl_maxpitch.value;
        }
        if cl.viewangles[PITCH] < cl_minpitch.value {
            cl.viewangles[PITCH] = cl_minpitch.value;
        }
    }
}

/// in_sdl.c `IN_MouseMove`.
unsafe fn mouse_move(cmd: &mut UserCmd) {
    // SAFETY: main thread; r_refdef is gl_rmain.c's live refdef_t
    unsafe {
        let deg2rad = |a: f32| f64::from(a) * (core::f64::consts::PI / 180.0);
        let mut sens = (libm::tan(deg2rad(r_refdef.basefov) * 0.5)
            / libm::tan(deg2rad(scr_fov.value) * 0.5)) as f32;
        sens *= sensitivity.value;

        let dmx = TOTAL_DX * sens;
        let dmy = TOTAL_DY * sens;

        TOTAL_DX = 0.0;
        TOTAL_DY = 0.0;

        // do pause check after resetting total_d* so mouse movements during
        // pause don't accumulate
        if cl.paused || key_dest != KEY_GAME {
            return;
        }

        let strafe = (in_strafe.state & 1) != 0;
        let mlook = (in_mlook.state & 1) != 0;

        if strafe || (lookstrafe.value != 0.0 && mlook) {
            cmd.sidemove_accumulator += m_side.value * dmx;
        } else {
            cl.viewangles[YAW] -= m_yaw.value * dmx;
        }

        if mlook && (dmx != 0.0 || dmy != 0.0) {
            V_StopPitchDrift();
        }

        if mlook && !strafe {
            cl.viewangles[PITCH] += m_pitch.value * dmy;
            /* johnfitz -- variable pitch clamping */
            if cl.viewangles[PITCH] > cl_maxpitch.value {
                cl.viewangles[PITCH] = cl_maxpitch.value;
            }
            if cl.viewangles[PITCH] < cl_minpitch.value {
                cl.viewangles[PITCH] = cl_minpitch.value;
            }
        } else if strafe && noclip_anglehack {
            cmd.upmove_accumulator -= m_forward.value * dmy;
        } else {
            cmd.forwardmove_accumulator -= m_forward.value * dmy;
        }
    }
}

/// in_sdl.c `IN_Move`.
///
/// # Safety
/// Main thread, after `VID_Init` (mirrors the C entry points' contract).
pub unsafe fn r#move(cmd: *mut UserCmd) {
    // SAFETY: caller passes cl.pendingcmd (cl_main.c:1054), live and
    // unaliased for the call
    unsafe {
        let cmd = &mut *cmd;
        // We only want the latest joystick movements
        cmd.forwardmove = 0.0;
        cmd.sidemove = 0.0;
        cmd.upmove = 0.0;

        joy_move(cmd);
        mouse_move(cmd);
    }
}

/// in_sdl.c `IN_UpdateInputMode`.
///
/// # Safety
/// Main thread, after `VID_Init` (mirrors the C entry points' contract).
pub unsafe fn update_input_mode() {
    // SAFETY: main thread; see the static invariant
    unsafe {
        let want_textmode = Key_TextEntry();
        if TEXTMODE != want_textmode {
            TEXTMODE = want_textmode;
            let debug = in_debugkeys.value != 0.0;
            if debug {
                sys::Con_Printf(
                    c"SDL_EnableUNICODE %d time: %g\n".as_ptr(),
                    c_int::from(TEXTMODE),
                    sys::Sys_DoubleTime(),
                );
            }
            if TEXTMODE {
                backend::start_text_input();
                if debug {
                    sys::Con_Printf(
                        c"SDL_StartTextInput time: %g\n".as_ptr(),
                        sys::Sys_DoubleTime(),
                    );
                }
            } else {
                backend::stop_text_input();
                if debug {
                    sys::Con_Printf(
                        c"SDL_StopTextInput time: %g\n".as_ptr(),
                        sys::Sys_DoubleTime(),
                    );
                }
            }
        }
    }
}

/* ---------------------------------------------------------------------------
 * Event pump (in_sdl3.c / in_sdl2.c IN_SendKeyEvents).
 */

/// in_sdl3.c `IN_DebugTextEvent`.
unsafe fn debug_text_event(text: &CStr) {
    // SAFETY: format string and argument agree
    unsafe {
        sys::Con_Printf(
            c"SDL_TEXTINPUT '%s' time: %g\n".as_ptr(),
            text.as_ptr(),
            sys::Sys_DoubleTime(),
        );
    }
}

/// in_sdl3.c `IN_DebugKeyEvent`.
unsafe fn debug_key_event(down: bool, scancode: c_int, keycode: c_int) {
    // SAFETY: SDL_Get*Name return static strings; arguments match the format
    unsafe {
        let eventtype: &CStr = if down { c"SDL_KEYDOWN" } else { c"SDL_KEYUP" };
        sys::Con_Printf(
            c"%s scancode: '%s' keycode: '%s' time: %g\n".as_ptr(),
            eventtype.as_ptr(),
            backend::scancode_name(scancode),
            backend::key_name(keycode),
            sys::Sys_DoubleTime(),
        );
    }
}

/// One `switch (event.type)` arm of `IN_SendKeyEvents`. Gamepad
/// add/remove/remap are handled inside the backend pump (they touch only the
/// backend's handle and `Con_DPrintf`).
unsafe fn handle_event(event: Event) -> Raise {
    // SAFETY: main thread, inside SDL_PollEvent's loop; the guarded callees
    // return their Host_Guard status
    unsafe {
        match event {
            Event::FocusGained => {
                S_UnblockSound();
                VID_FocusGained();
            }
            Event::FocusLost => {
                S_BlockSound();
                VID_FocusLost();
            }
            Event::SizeChanged(w, h) => {
                vid.width = w;
                vid.height = h;
                vid.restart_next_frame = true;
                raise!(InSdl_Glue_ConscaleCallback());
            }
            Event::TextInput(text) => {
                if in_debugkeys.value != 0.0 {
                    debug_text_event(text);
                }
                for &ch in text.to_bytes() {
                    if (ch & !0x7F) == 0 {
                        raise!(InSdl_Glue_CharEvent(c_int::from(ch)));
                    }
                }
            }
            Event::Key {
                down,
                scancode,
                keycode,
            } => {
                if in_debugkeys.value != 0.0 {
                    debug_key_event(down, scancode, keycode);
                }
                let key = scancode::scancode_to_quake_key(scancode);
                raise!(InSdl_Glue_KeyEventWithKeycode(key, down, keycode));
            }
            Event::MouseButton { button, down, x, y } => {
                if !(1..=5).contains(&button) {
                    sys::Con_Printf(c"Ignored event for mouse button %d\n".as_ptr(), button);
                } else {
                    if key_dest == KEY_CONSOLE {
                        let (cx, cy) = scale_mouse_coords(x, y);
                        Con_Mousemove(cx, cy);
                    }
                    raise!(InSdl_Glue_KeyEvent(
                        BUTTONREMAP[(button - 1) as usize],
                        down
                    ));
                }
            }
            Event::MouseWheel(y) => {
                if y > 0.0 {
                    raise!(InSdl_Glue_KeyEvent(K_MWHEELUP, true));
                    raise!(InSdl_Glue_KeyEvent(K_MWHEELUP, false));
                } else if y < 0.0 {
                    raise!(InSdl_Glue_KeyEvent(K_MWHEELDOWN, true));
                    raise!(InSdl_Glue_KeyEvent(K_MWHEELDOWN, false));
                }
            }
            Event::MouseMotion(xrel, yrel) => mouse_motion(xrel, yrel),
            Event::Quit => {
                raise!(InSdl_Glue_CLDisconnect());
                raise!(InSdl_Glue_SysQuit());
            }
        }
    }
    0
}

/// in_sdl3.c / in_sdl2.c `IN_SendKeyEvents`: drain the SDL queue.
///
/// # Safety
/// Main thread, after `VID_Init` (mirrors the C entry points' contract).
pub unsafe fn send_key_events() -> Raise {
    // SAFETY: main thread
    unsafe { backend::poll_events(&mut |event| handle_event(event)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_keeps_signs_and_zero() {
        assert_eq!(apply_easing(JoyAxis::default(), 2.0), JoyAxis::default());
        let a = apply_easing(JoyAxis { x: -0.5, y: 0.0 }, 2.0);
        assert!((a.x + 0.25).abs() < 1e-6 && a.y == 0.0);
        let b = apply_easing(JoyAxis { x: 0.0, y: 1.0 }, 3.0);
        assert!((b.y - 1.0).abs() < 1e-6);
    }

    #[test]
    fn deadzone_clamps_and_rescales() {
        assert_eq!(
            apply_deadzone(JoyAxis { x: 0.1, y: 0.1 }, 0.175, 0.02),
            JoyAxis::default()
        );
        let full = apply_deadzone(JoyAxis { x: 1.0, y: 0.0 }, 0.175, 0.02);
        assert!((full.x - 1.0).abs() < 1e-6, "{full:?}");
        let mid = apply_deadzone(JoyAxis { x: 0.5, y: 0.0 }, 0.175, 0.02);
        let expect = ((0.5f32 - 0.175) as f64 / (1.0 - 0.175 - 0.02)) as f32;
        assert!((mid.x - expect).abs() < 1e-6, "{mid:?} vs {expect}");
    }

    #[test]
    fn controller_button_keys() {
        assert_eq!(key_for_controller_button(0), K_ABUTTON);
        assert_eq!(key_for_controller_button(5), 0);
        assert_eq!(key_for_controller_button(14), K_RIGHTARROW);
        assert_eq!(key_for_controller_button(20), K_TOUCHPAD);
        assert_eq!(key_for_controller_button(21), 0);
        assert_eq!(key_for_controller_button(25), 0);
    }
}
