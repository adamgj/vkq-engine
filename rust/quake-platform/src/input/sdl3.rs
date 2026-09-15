//! `Quake/in_sdl3.c` and the `USE_SDL3` arms of `in_sdl.c`: the SDL3 calls
//! behind `input`. Owns the active gamepad (`joy_active_controller` /
//! `joy_active_instaceid`) and the mouse-event filter.

use core::ffi::{c_char, c_int, c_void, CStr};
use core::ptr;

use quake_c_sys as sys;
use sdl3::sys::events::{
    SDL_Event, SDL_EventFilter, SDL_GetEventFilter, SDL_PollEvent, SDL_SetEventFilter,
    SDL_EVENT_GAMEPAD_ADDED, SDL_EVENT_GAMEPAD_REMAPPED, SDL_EVENT_GAMEPAD_REMOVED,
    SDL_EVENT_KEY_DOWN, SDL_EVENT_KEY_UP, SDL_EVENT_MOUSE_BUTTON_DOWN, SDL_EVENT_MOUSE_BUTTON_UP,
    SDL_EVENT_MOUSE_MOTION, SDL_EVENT_MOUSE_WHEEL, SDL_EVENT_QUIT, SDL_EVENT_TEXT_INPUT,
    SDL_EVENT_WINDOW_FOCUS_GAINED, SDL_EVENT_WINDOW_FOCUS_LOST,
    SDL_EVENT_WINDOW_PIXEL_SIZE_CHANGED,
};
use sdl3::sys::gamepad::{
    SDL_AddGamepadMappingsFromFile, SDL_CloseGamepad, SDL_Gamepad, SDL_GamepadAxis,
    SDL_GamepadButton, SDL_GetGamepadAxis, SDL_GetGamepadButton, SDL_GetGamepadJoystick,
    SDL_GetGamepadNameForID, SDL_IsGamepad, SDL_OpenGamepad, SDL_GAMEPAD_BUTTON_COUNT,
};
use sdl3::sys::init::{SDL_InitSubSystem, SDL_QuitSubSystem, SDL_INIT_GAMEPAD};
use sdl3::sys::joystick::{SDL_GetJoystickID, SDL_GetJoysticks, SDL_JoystickID};
use sdl3::sys::keyboard::{
    SDL_GetKeyName, SDL_GetScancodeName, SDL_StartTextInput, SDL_StopTextInput,
};
use sdl3::sys::keycode::SDL_Keycode;
use sdl3::sys::mouse::{SDL_GetMouseState, SDL_SetWindowRelativeMouseMode};
use sdl3::sys::scancode::SDL_Scancode;
use sdl3::sys::stdinc::SDL_free;
use sdl3::sys::video::{SDL_GetWindowSize, SDL_Window};

use super::{Event, Raise};

/// `SDL_GAMEPAD_BUTTON_COUNT`.
pub(super) const BUTTON_COUNT: usize = SDL_GAMEPAD_BUTTON_COUNT.0 as usize;

pub(super) const RELATIVE_MOUSE_WARNING: &CStr =
    c"WARNING: SDL_SetWindowRelativeMouseMode(true) failed.\n";
pub(super) const GAMEPAD_INIT_WARNING: &CStr = c"could not initialize SDL Gamepad\n";

// SAFETY invariant: main thread only, like in_sdl3.c's statics; in SDL3, 0
// is the invalid joystick ID.
static mut JOY_ACTIVE_INSTANCEID: SDL_JoystickID = SDL_JoystickID(0);
static mut JOY_ACTIVE_CONTROLLER: *mut SDL_Gamepad = ptr::null_mut();

unsafe fn window() -> *mut SDL_Window {
    // SAFETY: VID_GetWindow returns the live SDL_Window * (or NULL before
    // VID_Init, which SDL tolerates)
    unsafe { sys::input::VID_GetWindow().cast() }
}

pub(super) unsafe fn start_text_input() {
    // SAFETY: plain SDL call on the main thread
    unsafe {
        SDL_StartTextInput(window());
    }
}

pub(super) unsafe fn stop_text_input() {
    // SAFETY: plain SDL call on the main thread
    unsafe {
        SDL_StopTextInput(window());
    }
}

/// `SDL_SetWindowRelativeMouseMode`; true on success.
pub(super) unsafe fn set_relative_mouse_mode(on: bool) -> bool {
    // SAFETY: plain SDL call on the main thread
    unsafe { SDL_SetWindowRelativeMouseMode(window(), on) }
}

#[cfg(target_os = "macos")]
pub(super) unsafe fn warp_mouse_to_center() {
    // SAFETY: plain SDL calls on the main thread
    unsafe {
        let (width, height) = window_size();
        sdl3::sys::mouse::SDL_WarpMouseInWindow(window(), (width / 2) as f32, (height / 2) as f32);
    }
}

pub(super) unsafe fn window_size() -> (c_int, c_int) {
    let mut w: c_int = 0;
    let mut h: c_int = 0;
    // SAFETY: out-pointers to locals; SDL leaves them at 0 on failure
    unsafe {
        SDL_GetWindowSize(window(), &mut w, &mut h);
    }
    (w, h)
}

pub(super) unsafe fn mouse_position() -> (f32, f32) {
    let mut x: f32 = 0.0;
    let mut y: f32 = 0.0;
    // SAFETY: out-pointers to locals
    unsafe {
        SDL_GetMouseState(&mut x, &mut y);
    }
    (x, y)
}

/// in_sdl3.c `IN_SDL_FilterMouseEvents`.
unsafe extern "C" fn filter_mouse_events(_userdata: *mut c_void, event: *mut SDL_Event) -> bool {
    // SAFETY: SDL hands a valid event; `r#type` is the union's first field
    unsafe {
        if (*event).r#type == SDL_EVENT_MOUSE_MOTION.0 {
            super::filter_mouse_motion((*event).motion.x, (*event).motion.y);
            return false;
        }
    }
    true
}

pub(super) unsafe fn begin_ignoring_mouse_events() {
    let mut current_filter: SDL_EventFilter = None;
    let mut current_userdata: *mut c_void = ptr::null_mut();
    // SAFETY: out-pointers to locals
    unsafe {
        SDL_GetEventFilter(&mut current_filter, &mut current_userdata);
        // Same address test as in_sdl3.c (`currentFilter != IN_SDL_FilterMouseEvents`).
        let installed = current_filter.is_some_and(|f| {
            core::ptr::fn_addr_eq(f, filter_mouse_events as unsafe extern "C" fn(_, _) -> bool)
        });
        if !installed {
            SDL_SetEventFilter(Some(filter_mouse_events), ptr::null_mut());
        }
    }
}

pub(super) unsafe fn end_ignoring_mouse_events() {
    let mut current_filter: SDL_EventFilter = None;
    let mut current_userdata: *mut c_void = ptr::null_mut();
    // SAFETY: out-pointers to locals
    unsafe {
        if SDL_GetEventFilter(&mut current_filter, &mut current_userdata) {
            SDL_SetEventFilter(None, ptr::null_mut());
        }
    }
}

pub(super) unsafe fn init_gamepad_subsystem() -> bool {
    // SAFETY: plain SDL call on the main thread
    unsafe { SDL_InitSubSystem(SDL_INIT_GAMEPAD) }
}

pub(super) unsafe fn add_gamepad_mappings_from_file(path: *const c_char) -> c_int {
    // SAFETY: `path` is NUL-terminated
    unsafe { SDL_AddGamepadMappingsFromFile(path) }
}

/// The enumerate-and-open tail of in_sdl3.c `IN_StartupJoystick`.
pub(super) unsafe fn open_first_gamepad() {
    // SAFETY: main thread; SDL_GetJoysticks returns an SDL-allocated array of
    // `count` ids (or NULL) that SDL_free releases
    unsafe {
        let mut count: c_int = 0;
        let joysticks = SDL_GetJoysticks(&mut count);
        if joysticks.is_null() {
            return;
        }
        for i in 0..count.max(0) as usize {
            let id = *joysticks.add(i);
            if SDL_IsGamepad(id) {
                let controllername = SDL_GetGamepadNameForID(id);
                let name = if controllername.is_null() {
                    c"NULL".as_ptr()
                } else {
                    controllername
                };
                let gamecontroller = SDL_OpenGamepad(id);
                if !gamecontroller.is_null() {
                    sys::Con_Printf(c"detected controller: %s\n".as_ptr(), name);
                    JOY_ACTIVE_INSTANCEID = id;
                    JOY_ACTIVE_CONTROLLER = gamecontroller;
                    SDL_free(joysticks.cast());
                    return;
                }
                sys::Con_Warning(c"failed to open controller: %s\n".as_ptr(), name);
            }
        }
        SDL_free(joysticks.cast());
    }
}

/// in_sdl3.c `IN_ShutdownJoystick`.
pub(super) unsafe fn shutdown_joystick() {
    // SAFETY: plain SDL call on the main thread
    unsafe {
        SDL_QuitSubSystem(SDL_INIT_GAMEPAD);
    }
}

pub(super) unsafe fn joystick_active() -> bool {
    // SAFETY: main thread; see the static invariant
    unsafe { !JOY_ACTIVE_CONTROLLER.is_null() }
}

pub(super) unsafe fn gamepad_button(i: usize) -> bool {
    // SAFETY: caller checked joystick_active(); i < BUTTON_COUNT
    unsafe { SDL_GetGamepadButton(JOY_ACTIVE_CONTROLLER, SDL_GamepadButton(i as c_int)) }
}

pub(super) unsafe fn gamepad_axis(i: usize) -> i16 {
    // SAFETY: caller checked joystick_active(); i < AXIS_COUNT
    unsafe { SDL_GetGamepadAxis(JOY_ACTIVE_CONTROLLER, SDL_GamepadAxis(i as c_int)) }
}

pub(super) unsafe fn scancode_name(scancode: c_int) -> *const c_char {
    // SAFETY: plain SDL call; returns a static string
    unsafe { SDL_GetScancodeName(SDL_Scancode(scancode)) }
}

pub(super) unsafe fn key_name(keycode: c_int) -> *const c_char {
    // SAFETY: plain SDL call; returns a static string
    unsafe { SDL_GetKeyName(SDL_Keycode(keycode as u32)) }
}

/// in_sdl3.c `IN_SendKeyEvents`: the `SDL_PollEvent` loop. Gamepad
/// add/remove/remap arms are handled here; every other arm is translated
/// into an [`Event`] for `handler`, whose non-zero status ends the loop.
pub(super) unsafe fn poll_events(handler: &mut dyn FnMut(Event<'_>) -> Raise) -> Raise {
    // SAFETY: main thread; each union member is read only under its own
    // event type
    unsafe {
        let mut event: SDL_Event = core::mem::zeroed();
        while SDL_PollEvent(&mut event) {
            let ty = event.r#type;
            let translated = if ty == SDL_EVENT_WINDOW_FOCUS_GAINED.0 {
                Event::FocusGained
            } else if ty == SDL_EVENT_WINDOW_FOCUS_LOST.0 {
                Event::FocusLost
            } else if ty == SDL_EVENT_WINDOW_PIXEL_SIZE_CHANGED.0 {
                // data1/data2 are in pixels, matching vid.width/height, and
                // this also fires when only the display scale changes
                Event::SizeChanged(event.window.data1, event.window.data2)
            } else if ty == SDL_EVENT_TEXT_INPUT.0 {
                Event::TextInput(CStr::from_ptr(event.text.text))
            } else if ty == SDL_EVENT_KEY_DOWN.0 || ty == SDL_EVENT_KEY_UP.0 {
                Event::Key {
                    down: event.key.down,
                    scancode: event.key.scancode.0,
                    keycode: event.key.key.0 as c_int,
                }
            } else if ty == SDL_EVENT_MOUSE_BUTTON_DOWN.0 || ty == SDL_EVENT_MOUSE_BUTTON_UP.0 {
                Event::MouseButton {
                    button: c_int::from(event.button.button),
                    down: event.button.down,
                    x: event.button.x,
                    y: event.button.y,
                }
            } else if ty == SDL_EVENT_MOUSE_WHEEL.0 {
                Event::MouseWheel(event.wheel.y)
            } else if ty == SDL_EVENT_MOUSE_MOTION.0 {
                Event::MouseMotion(event.motion.xrel, event.motion.yrel)
            } else if ty == SDL_EVENT_GAMEPAD_ADDED.0 {
                if JOY_ACTIVE_INSTANCEID.0 == 0 {
                    JOY_ACTIVE_CONTROLLER = SDL_OpenGamepad(event.gdevice.which);
                    if JOY_ACTIVE_CONTROLLER.is_null() {
                        sys::Con_DPrintf(c"Couldn't open game controller\n".as_ptr());
                    } else {
                        let joy = SDL_GetGamepadJoystick(JOY_ACTIVE_CONTROLLER);
                        JOY_ACTIVE_INSTANCEID = SDL_GetJoystickID(joy);
                    }
                } else {
                    sys::Con_DPrintf(c"Ignoring SDL_EVENT_GAMEPAD_ADDED\n".as_ptr());
                }
                continue;
            } else if ty == SDL_EVENT_GAMEPAD_REMOVED.0 {
                if JOY_ACTIVE_INSTANCEID.0 != 0 && event.gdevice.which == JOY_ACTIVE_INSTANCEID {
                    SDL_CloseGamepad(JOY_ACTIVE_CONTROLLER);
                    JOY_ACTIVE_CONTROLLER = ptr::null_mut();
                    JOY_ACTIVE_INSTANCEID = SDL_JoystickID(0);
                } else {
                    sys::Con_DPrintf(c"Ignoring SDL_EVENT_GAMEPAD_REMOVED\n".as_ptr());
                }
                continue;
            } else if ty == SDL_EVENT_GAMEPAD_REMAPPED.0 {
                sys::Con_DPrintf(c"Ignoring SDL_EVENT_GAMEPAD_REMAPPED\n".as_ptr());
                continue;
            } else if ty == SDL_EVENT_QUIT.0 {
                Event::Quit
            } else {
                continue;
            };
            let r = handler(translated);
            if r != 0 {
                return r;
            }
        }
    }
    0
}
