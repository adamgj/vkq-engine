//! `Quake/main_sdl.c` (Rust migration Phase 9 M6, ADR-017): the process
//! entry -- `parms` setup, `Sys_InitSDL`, `Sys_Init`, the startup banner,
//! `Host_Init` and the dedicated / client frame loops.
//!
//! [`quake_main`] is the body of the C `main`; the exported entry symbols
//! (`main`, and on Windows `WinMain` / `SDL_main`) live in
//! `quake-capi::entry` so the platform crate has no `#[no_mangle]` exports.
//!
//! Raising (ADR-009): `Sys_Init`, `Host_Init` and `Host_Frame` are called
//! through `Host_Guard` trampolines in `Quake/sys_glue.c`, and their status
//! is consumed here as a [`HostError`] (Phase 9 M7). A frame that raises
//! just ends -- what `Host_Glue_FrameInner`'s `setjmp` early return did
//! before M6 (the outermost guard now sits in this loop; under
//! `USE_RUST_PLATFORM` the C frame wrapper is itself a `Host_Guard` whose
//! status `quake_rs_host_frame` hands back, so no jump crosses it; see
//! [`recover`]). A raise during
//! `Sys_Init` / `Host_Init` used to land on an un-`setjmp`ed
//! `host_abortserver` (undefined behaviour in C); it is now a `Sys_Error`.

use core::ffi::{c_char, c_int};
use core::ptr;

use quake_c_sys::host::{host_parms, no_rendering, quakeparms_t, sys_ticrate};
use quake_c_sys::sys as c;
use quake_c_sys::{harness_active, isDedicated, listening, COM_CheckParm};
use quake_host::error::HostError;

#[cfg(feature = "sdl2")]
use sdl2::sys::{SDL_Delay, SDL_GetError, SDL_Init, SDL_Quit};
#[cfg(feature = "sdl3")]
use sdl3::sys::{
    error::SDL_GetError,
    init::{SDL_Init, SDL_InitFlags, SDL_Quit},
    timer::SDL_Delay,
};

use crate::sys::{double_time, sys_error, sys_printf};

/// `static void Sys_AtExit (void) { SDL_Quit (); }`
unsafe extern "C" fn sys_at_exit() {
    // SAFETY: plain SDL call from the C runtime's exit sequence.
    unsafe { SDL_Quit() }
}

/// `SDL_GetError ()` as text for a `Sys_Error` message.
unsafe fn sdl_error() -> String {
    // SAFETY: SDL's error string is a valid NUL-terminated string (possibly
    // empty) until the next SDL call on this thread.
    unsafe {
        let p = SDL_GetError();
        if p.is_null() {
            String::new()
        } else {
            core::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

/// C: `static void Sys_InitSDL (void)`
unsafe fn init_sdl() {
    #[cfg(feature = "sdl3")]
    let (major, minor, patch) = {
        use sdl3::sys::version::{
            SDL_GetVersion, SDL_VERSIONNUM_MAJOR, SDL_VERSIONNUM_MICRO, SDL_VERSIONNUM_MINOR,
        };
        let version = SDL_GetVersion();
        (
            SDL_VERSIONNUM_MAJOR(version),
            SDL_VERSIONNUM_MINOR(version),
            SDL_VERSIONNUM_MICRO(version),
        )
    };
    #[cfg(feature = "sdl2")]
    let (major, minor, patch) = {
        use sdl2::sys::{SDL_GetVersion, SDL_version};
        let mut version = SDL_version {
            major: 0,
            minor: 0,
            patch: 0,
        };
        // SAFETY: `version` outlives the call.
        unsafe { SDL_GetVersion(&mut version) };
        (
            c_int::from(version.major),
            c_int::from(version.minor),
            c_int::from(version.patch),
        )
    };

    sys_printf(&format!("Using SDL version {major}.{minor}.{patch}\n"));

    // SAFETY: plain SDL calls before any other SDL use.
    unsafe {
        #[cfg(feature = "sdl3")]
        let initialized = SDL_Init(SDL_InitFlags(0));
        #[cfg(feature = "sdl2")]
        let initialized = SDL_Init(0) >= 0;

        if !initialized {
            sys_error(&format!("Couldn't init SDL: {}", sdl_error()));
        }

        #[cfg(feature = "engine-debug")]
        {
            #[cfg(feature = "sdl3")]
            sdl3::sys::log::SDL_SetLogPriorities(sdl3::sys::log::SDL_LOG_PRIORITY_DEBUG);
            #[cfg(feature = "sdl2")]
            sdl2::sys::SDL_LogSetAllPriority(sdl2::sys::SDL_LogPriority::SDL_LOG_PRIORITY_DEBUG);
        }

        c::atexit(Some(sys_at_exit));
    }
}

/// `SDL_GetNumLogicalCPUCores ()` (SDL3) / `SDL_GetCPUCount ()` (SDL2).
unsafe fn cpu_count() -> c_int {
    // SAFETY: plain SDL call after `SDL_Init`.
    unsafe {
        #[cfg(feature = "sdl3")]
        {
            sdl3::sys::cpuinfo::SDL_GetNumLogicalCPUCores()
        }
        #[cfg(feature = "sdl2")]
        {
            sdl2::sys::SDL_GetCPUCount()
        }
    }
}

/// `static quakeparms_t parms;`
static mut PARMS: quakeparms_t = quakeparms_t {
    basedir: ptr::null(),
    userdir: ptr::null(),
    argc: 0,
    argv: ptr::null_mut(),
    errstate: 0,
};

/// `Host_Frame (time)` under the guard; a raised frame ends in [`recover`].
unsafe fn host_frame(time: f64) {
    // SAFETY: main thread, after `Host_Init`.
    let status = unsafe { c::SysGlue_HostFrame(time) };
    if let Err(err) = HostError::from_guard_status(status) {
        recover(err);
    }
}

/// Frame-abort recovery: the sole owner of it under `USE_RUST_PLATFORM`
/// (Phase 9 D7), what `_Host_Frame`'s `setjmp (host_abortserver)` early
/// return did in the C. For `AbortServer`, `Host_Error`/`Host_EndGame`
/// have already shut the server down and disconnected the client, so by
/// ADR-009's post-guard invariant nothing here touches that state; for
/// `ScreenError` the jump was taken before either, and the loop continues
/// with the server and client as they were, as `SCR_DrawGUI`'s own recovery
/// point does. A status `Host_Guard` does not define has no C precedent
/// (its `setjmp`s return 1) and is reported instead of ignored.
fn recover(err: HostError) {
    match err {
        HostError::AbortServer | HostError::ScreenError => {}
        HostError::Unknown(_) => sys_error(&format!("Host_Frame: {err}")),
    }
}

/// `SDL_RunApp (0, NULL, SDL_main, NULL)` -- the body of `SDL3/SDL_main.h`'s
/// Windows `WinMain` shim: SDL builds the UTF-8 `argc`/`argv` from the wide
/// command line and calls [`quake_main`] with them.
///
/// # Safety
/// The process entry (`WinMain`), once on the main thread.
#[cfg(all(windows, feature = "sdl3"))]
pub unsafe fn run_app() -> c_int {
    // SAFETY: caller contract; SDL accepts 0/NULL and derives the arguments.
    unsafe { sdl3::sys::main::SDL_RunApp(0, ptr::null_mut(), Some(quake_main), ptr::null_mut()) }
}

/// C: `int main (int argc, char *argv[])` -- everything after the entry
/// symbol. Never returns in practice (`Sys_Quit` / `Sys_Error` `exit`), the
/// `return 0` of the C original is unreachable behind the two loops.
///
/// # Safety
/// The process entry, called once on the main thread with the C runtime's
/// `argc`/`argv`.
pub unsafe extern "C" fn quake_main(argc: c_int, argv: *mut *mut c_char) -> c_int {
    // SAFETY: caller contract -- single-threaded startup; the statics are
    // the engine's own globals, written before anything reads them.
    unsafe {
        host_parms = ptr::addr_of_mut!(PARMS);
        PARMS.basedir = c".".as_ptr();

        PARMS.argc = argc;
        PARMS.argv = argv;

        PARMS.errstate = 0;

        c::COM_InitArgv(PARMS.argc, PARMS.argv);

        isDedicated = COM_CheckParm(c"-dedicated".as_ptr()) != 0;
        c::Harness_CheckArgs();

        init_sdl();

        if HostError::from_guard_status(c::SysGlue_SysInit()).is_err() {
            sys_error("Sys_Init: Host_Error during startup");
        }

        sys_printf(&format!("Detected {} CPUs.\n", cpu_count()));
        sys_printf(&format!(
            "Initializing {}\n",
            core::ffi::CStr::from_ptr(c::SysGlue_EngineNameAndVer()).to_string_lossy()
        ));
        c::SysGlue_PrintCompilerBanner();

        sys_printf("Host_Init\n");
        if HostError::from_guard_status(c::SysGlue_HostInit()).is_err() {
            sys_error("Host_Init: Host_Error during startup");
        }

        let mut oldtime = double_time();
        if isDedicated {
            loop {
                let mut newtime = double_time();
                let mut time = newtime - oldtime;

                while time < f64::from(sys_ticrate.value) {
                    SDL_Delay(1);
                    newtime = double_time();
                    time = newtime - oldtime;
                }

                /* fixed timestep: state must not depend on wall-clock time */
                if c::harness_fixed_dt {
                    time = c::Harness_FrameTime();
                }

                host_frame(time);
                oldtime = newtime;
            }
        } else {
            loop {
                if !no_rendering {
                    /* If we have no input focus at all, sleep a bit */
                    if (!listening && !c::VID_HasMouseOrInputFocus()) || c::SysGlue_ClientPaused() {
                        SDL_Delay(16);
                    }
                    /* If we're minimised, sleep a bit more */
                    if !listening && c::VID_IsMinimized() {
                        SDL_Delay(32);
                    }
                }

                /* A harness client with no fixed timestep (-headless alone,
                e.g. interop_matrix.py's live network client) is one half
                of a genuine two-process network session, not a hash
                subject. Unthrottled it races through -exitafter frames as
                fast as the CPU allows and floods the dedicated server's
                unreliable channel faster than the server (paced to
                sys_ticrate) drains its socket, and OS receive-buffer drop
                timing is then nondeterministic across launches. Pacing
                the client to the server's real-time cadence keeps the
                packet exchange bounded and reproducible; the predicate
                is deliberately wider than that one script (it also paces
                builtin_diff's and capture_session's runs). The floor is
                sys_ticrate, nominally the *dedicated server* tic rate. A
                timedemo is exempt: it has no server and no peer, and the
                floor would cap the fps it exists to measure. */
                let mut newtime = double_time();
                let mut time = newtime - oldtime;
                if harness_active && !c::harness_fixed_dt && !c::SysGlue_ClientTimedemo() {
                    while time < f64::from(sys_ticrate.value) {
                        SDL_Delay(1);
                        newtime = double_time();
                        time = newtime - oldtime;
                    }
                }

                /* fixed timestep: state must not depend on wall-clock time */
                if c::harness_fixed_dt {
                    time = c::Harness_FrameTime();
                }

                host_frame(time);

                oldtime = newtime;
            }
        }
    }
}
