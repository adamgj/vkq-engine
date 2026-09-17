//! `Quake/in_sdl2.c` and the SDL2 arms of `in_sdl.c`: the SDL2 calls behind
//! `input`. Owns the active game controller (`joy_active_controller` /
//! `joy_active_instaceid`) and the mouse-event filter.
//!
//! Only type-checked locally (no SDL2 development box; the SDL2 leg is the
//! Linux/macOS CI matrix).

use core::ffi::{c_char, c_int, c_void, CStr};
use core::ptr;

use quake_c_sys as sys;
use sdl2::sys::{
    SDL_Event, SDL_EventFilter, SDL_EventType, SDL_GameController,
    SDL_GameControllerAddMappingsFromRW, SDL_GameControllerAxis, SDL_GameControllerButton,
    SDL_GameControllerClose, SDL_GameControllerGetAxis, SDL_GameControllerGetButton,
    SDL_GameControllerGetJoystick, SDL_GameControllerNameForIndex, SDL_GameControllerOpen,
    SDL_GetEventFilter, SDL_GetMouseState, SDL_GetWindowSize, SDL_InitSubSystem,
    SDL_IsGameController, SDL_JoystickID, SDL_JoystickInstanceID, SDL_JoystickNameForIndex,
    SDL_NumJoysticks, SDL_PollEvent, SDL_QuitSubSystem, SDL_RWFromFile, SDL_SetEventFilter,
    SDL_SetRelativeMouseMode, SDL_StartTextInput, SDL_StopTextInput, SDL_Window, SDL_WindowEventID,
    SDL_bool, SDL_INIT_GAMECONTROLLER, SDL_PRESSED,
};

use super::{Event, Raise};

/// `SDL_CONTROLLER_BUTTON_MAX`.
pub(super) const BUTTON_COUNT: usize = SDL_GameControllerButton::SDL_CONTROLLER_BUTTON_MAX as usize;

pub(super) const RELATIVE_MOUSE_WARNING: &CStr =
    c"WARNING: SDL_SetRelativeMouseMode(SDL_TRUE) failed.\n";
pub(super) const GAMEPAD_INIT_WARNING: &CStr = c"could not initialize SDL Game Controller\n";

/// `(SDL_GameControllerButton)i` for `i < SDL_CONTROLLER_BUTTON_MAX`; the
/// bindgen enum has no integer constructor.
const BUTTONS: [SDL_GameControllerButton; BUTTON_COUNT] = {
    #[allow(clippy::enum_glob_use)] // bindgen enum table, read like the C
    use SDL_GameControllerButton::*;
    [
        SDL_CONTROLLER_BUTTON_A,
        SDL_CONTROLLER_BUTTON_B,
        SDL_CONTROLLER_BUTTON_X,
        SDL_CONTROLLER_BUTTON_Y,
        SDL_CONTROLLER_BUTTON_BACK,
        SDL_CONTROLLER_BUTTON_GUIDE,
        SDL_CONTROLLER_BUTTON_START,
        SDL_CONTROLLER_BUTTON_LEFTSTICK,
        SDL_CONTROLLER_BUTTON_RIGHTSTICK,
        SDL_CONTROLLER_BUTTON_LEFTSHOULDER,
        SDL_CONTROLLER_BUTTON_RIGHTSHOULDER,
        SDL_CONTROLLER_BUTTON_DPAD_UP,
        SDL_CONTROLLER_BUTTON_DPAD_DOWN,
        SDL_CONTROLLER_BUTTON_DPAD_LEFT,
        SDL_CONTROLLER_BUTTON_DPAD_RIGHT,
        SDL_CONTROLLER_BUTTON_MISC1,
        SDL_CONTROLLER_BUTTON_PADDLE1,
        SDL_CONTROLLER_BUTTON_PADDLE2,
        SDL_CONTROLLER_BUTTON_PADDLE3,
        SDL_CONTROLLER_BUTTON_PADDLE4,
        SDL_CONTROLLER_BUTTON_TOUCHPAD,
    ]
};

/// `(SDL_GameControllerAxis)i` for `i < SDL_CONTROLLER_AXIS_MAX`.
const AXES: [SDL_GameControllerAxis; 6] = {
    #[allow(clippy::enum_glob_use)] // bindgen enum table, read like the C
    use SDL_GameControllerAxis::*;
    [
        SDL_CONTROLLER_AXIS_LEFTX,
        SDL_CONTROLLER_AXIS_LEFTY,
        SDL_CONTROLLER_AXIS_RIGHTX,
        SDL_CONTROLLER_AXIS_RIGHTY,
        SDL_CONTROLLER_AXIS_TRIGGERLEFT,
        SDL_CONTROLLER_AXIS_TRIGGERRIGHT,
    ]
};

extern "C" {
    // The bindgen prototypes take the Rust enums; the C ABI is `int`, and
    // the debug print may see scancodes/keycodes with no enum variant, so
    // they are re-declared with the underlying integer types.
    fn SDL_GetScancodeName(scancode: c_int) -> *const c_char;
    fn SDL_GetKeyName(key: i32) -> *const c_char;
}

// SAFETY invariant: main thread only, like in_sdl2.c's statics; -1 is the
// invalid joystick instance id.
static mut JOY_ACTIVE_INSTANCEID: SDL_JoystickID = -1;
static mut JOY_ACTIVE_CONTROLLER: *mut SDL_GameController = ptr::null_mut();

unsafe fn window() -> *mut SDL_Window {
    // SAFETY: VID_GetWindow returns the live SDL_Window * (or NULL before
    // VID_Init, which SDL tolerates)
    unsafe { sys::input::VID_GetWindow().cast() }
}

pub(super) unsafe fn start_text_input() {
    // SAFETY: plain SDL call on the main thread
    unsafe { SDL_StartTextInput() }
}

pub(super) unsafe fn stop_text_input() {
    // SAFETY: plain SDL call on the main thread
    unsafe { SDL_StopTextInput() }
}

/// `SDL_SetRelativeMouseMode`; true on success (C checks `!= 0`).
pub(super) unsafe fn set_relative_mouse_mode(on: bool) -> bool {
    let flag = if on {
        SDL_bool::SDL_TRUE
    } else {
        SDL_bool::SDL_FALSE
    };
    // SAFETY: plain SDL call on the main thread
    unsafe { SDL_SetRelativeMouseMode(flag) == 0 }
}

#[cfg(target_os = "macos")]
pub(super) unsafe fn warp_mouse_to_center() {
    // SAFETY: plain SDL calls on the main thread
    unsafe {
        let (width, height) = window_size();
        sdl2::sys::SDL_WarpMouseInWindow(window(), width / 2, height / 2);
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
    let mut x: c_int = 0;
    let mut y: c_int = 0;
    // SAFETY: out-pointers to locals
    unsafe {
        SDL_GetMouseState(&mut x, &mut y);
    }
    (x as f32, y as f32)
}

/// in_sdl2.c `IN_SDL_FilterMouseEvents`.
unsafe extern "C" fn filter_mouse_events(_userdata: *mut c_void, event: *mut SDL_Event) -> c_int {
    // SAFETY: SDL hands a valid event; `type_` is the union's first field
    unsafe {
        if (*event).type_ == SDL_EventType::SDL_MOUSEMOTION as u32 {
            super::filter_mouse_motion((*event).motion.x as f32, (*event).motion.y as f32);
            return 0;
        }
    }
    1
}

pub(super) unsafe fn begin_ignoring_mouse_events() {
    let mut current_filter: SDL_EventFilter = None;
    let mut current_userdata: *mut c_void = ptr::null_mut();
    // SAFETY: out-pointers to locals
    unsafe {
        SDL_GetEventFilter(&mut current_filter, &mut current_userdata);
        // Same address test as in_sdl2.c (`currentFilter != IN_SDL_FilterMouseEvents`).
        let installed = current_filter.is_some_and(|f| {
            core::ptr::fn_addr_eq(
                f,
                filter_mouse_events as unsafe extern "C" fn(_, _) -> c_int,
            )
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
        if SDL_GetEventFilter(&mut current_filter, &mut current_userdata) == SDL_bool::SDL_TRUE {
            SDL_SetEventFilter(None, ptr::null_mut());
        }
    }
}

pub(super) unsafe fn init_gamepad_subsystem() -> bool {
    // SAFETY: plain SDL call on the main thread
    unsafe { SDL_InitSubSystem(SDL_INIT_GAMECONTROLLER) != -1 }
}

/// The `SDL_GameControllerAddMappingsFromFile` macro.
pub(super) unsafe fn add_gamepad_mappings_from_file(path: *const c_char) -> c_int {
    // SAFETY: `path` is NUL-terminated; freerw=1 hands the RWops to SDL
    unsafe { SDL_GameControllerAddMappingsFromRW(SDL_RWFromFile(path, c"rb".as_ptr()), 1) }
}

/// The enumerate-and-open tail of in_sdl2.c `IN_StartupJoystick`.
pub(super) unsafe fn open_first_gamepad() {
    // SAFETY: main thread; the *ForIndex names are SDL-owned strings
    unsafe {
        for i in 0..SDL_NumJoysticks() {
            let joyname = SDL_JoystickNameForIndex(i);
            if SDL_IsGameController(i) == SDL_bool::SDL_TRUE {
                let controllername = SDL_GameControllerNameForIndex(i);
                let name = if controllername.is_null() {
                    c"NULL".as_ptr()
                } else {
                    controllername
                };
                let gamecontroller = SDL_GameControllerOpen(i);
                if !gamecontroller.is_null() {
                    sys::Con_Printf(c"detected controller: %s\n".as_ptr(), name);
                    JOY_ACTIVE_INSTANCEID =
                        SDL_JoystickInstanceID(SDL_GameControllerGetJoystick(gamecontroller));
                    JOY_ACTIVE_CONTROLLER = gamecontroller;
                    break;
                }
                sys::Con_Warning(c"failed to open controller: %s\n".as_ptr(), name);
            } else {
                let name = if joyname.is_null() {
                    c"NULL".as_ptr()
                } else {
                    joyname
                };
                sys::Con_Warning(c"joystick missing controller mappings: %s\n".as_ptr(), name);
            }
        }
    }
}

/// in_sdl2.c `IN_ShutdownJoystick`.
pub(super) unsafe fn shutdown_joystick() {
    // SAFETY: plain SDL call on the main thread
    unsafe { SDL_QuitSubSystem(SDL_INIT_GAMECONTROLLER) }
}

pub(super) unsafe fn joystick_active() -> bool {
    // SAFETY: main thread; see the static invariant
    unsafe { !JOY_ACTIVE_CONTROLLER.is_null() }
}

pub(super) unsafe fn gamepad_button(i: usize) -> bool {
    // SAFETY: caller checked joystick_active(); i < BUTTON_COUNT
    unsafe { SDL_GameControllerGetButton(JOY_ACTIVE_CONTROLLER, BUTTONS[i]) != 0 }
}

pub(super) unsafe fn gamepad_axis(i: usize) -> i16 {
    // SAFETY: caller checked joystick_active(); i < AXIS_COUNT
    unsafe { SDL_GameControllerGetAxis(JOY_ACTIVE_CONTROLLER, AXES[i]) }
}

pub(super) unsafe fn scancode_name(scancode: c_int) -> *const c_char {
    // SAFETY: plain SDL call; returns a static string
    unsafe { SDL_GetScancodeName(scancode) }
}

pub(super) unsafe fn key_name(keycode: c_int) -> *const c_char {
    // SAFETY: plain SDL call; returns a static string
    unsafe { SDL_GetKeyName(keycode) }
}

/// in_sdl2.c `IN_SendKeyEvents`: the `SDL_PollEvent` loop. Controller
/// add/remove/remap arms are handled here; every other arm is translated
/// into an [`Event`] for `handler`, whose non-zero status ends the loop.
pub(super) unsafe fn poll_events(handler: &mut dyn FnMut(Event<'_>) -> Raise) -> Raise {
    #[allow(clippy::enum_glob_use)] // bindgen enum table, read like the C
    use SDL_EventType::*;
    // SAFETY: main thread; each union member is read only under its own
    // event type
    unsafe {
        let mut event: SDL_Event = core::mem::zeroed();
        while SDL_PollEvent(&mut event) != 0 {
            let ty = event.type_;
            let translated = if ty == SDL_WINDOWEVENT as u32 {
                let which = event.window.event;
                if which == SDL_WindowEventID::SDL_WINDOWEVENT_FOCUS_GAINED as u8 {
                    Event::FocusGained
                } else if which == SDL_WindowEventID::SDL_WINDOWEVENT_FOCUS_LOST as u8 {
                    Event::FocusLost
                } else if which == SDL_WindowEventID::SDL_WINDOWEVENT_SIZE_CHANGED as u8 {
                    Event::SizeChanged(event.window.data1, event.window.data2)
                } else {
                    continue;
                }
            } else if ty == SDL_TEXTINPUT as u32 {
                let bytes: &[u8] = core::slice::from_raw_parts(
                    event.text.text.as_ptr().cast(),
                    event.text.text.len(),
                );
                // SDL guarantees NUL termination inside the 32-byte buffer
                let text = CStr::from_bytes_until_nul(bytes).unwrap_or(c"");
                Event::TextInput(text)
            } else if ty == SDL_KEYDOWN as u32 || ty == SDL_KEYUP as u32 {
                Event::Key {
                    down: u32::from(event.key.state) == SDL_PRESSED,
                    scancode: event.key.keysym.scancode as c_int,
                    keycode: event.key.keysym.sym,
                }
            } else if ty == SDL_MOUSEBUTTONDOWN as u32 || ty == SDL_MOUSEBUTTONUP as u32 {
                Event::MouseButton {
                    button: c_int::from(event.button.button),
                    down: u32::from(event.button.state) == SDL_PRESSED,
                    x: event.button.x as f32,
                    y: event.button.y as f32,
                }
            } else if ty == SDL_MOUSEWHEEL as u32 {
                Event::MouseWheel(event.wheel.y as f32)
            } else if ty == SDL_MOUSEMOTION as u32 {
                Event::MouseMotion(event.motion.xrel as f32, event.motion.yrel as f32)
            } else if ty == SDL_CONTROLLERDEVICEADDED as u32 {
                if JOY_ACTIVE_INSTANCEID == -1 {
                    JOY_ACTIVE_CONTROLLER = SDL_GameControllerOpen(event.cdevice.which);
                    if JOY_ACTIVE_CONTROLLER.is_null() {
                        sys::Con_DPrintf(c"Couldn't open game controller\n".as_ptr());
                    } else {
                        let joy = SDL_GameControllerGetJoystick(JOY_ACTIVE_CONTROLLER);
                        JOY_ACTIVE_INSTANCEID = SDL_JoystickInstanceID(joy);
                    }
                } else {
                    sys::Con_DPrintf(c"Ignoring SDL_CONTROLLERDEVICEADDED\n".as_ptr());
                }
                continue;
            } else if ty == SDL_CONTROLLERDEVICEREMOVED as u32 {
                if JOY_ACTIVE_INSTANCEID != -1 && event.cdevice.which == JOY_ACTIVE_INSTANCEID {
                    SDL_GameControllerClose(JOY_ACTIVE_CONTROLLER);
                    JOY_ACTIVE_CONTROLLER = ptr::null_mut();
                    JOY_ACTIVE_INSTANCEID = -1;
                } else {
                    sys::Con_DPrintf(c"Ignoring SDL_CONTROLLERDEVICEREMOVED\n".as_ptr());
                }
                continue;
            } else if ty == SDL_CONTROLLERDEVICEREMAPPED as u32 {
                sys::Con_DPrintf(c"Ignoring SDL_CONTROLLERDEVICEREMAPPED\n".as_ptr());
                continue;
            } else if ty == SDL_QUIT as u32 {
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
