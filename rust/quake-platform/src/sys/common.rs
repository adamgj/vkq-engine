//! `Quake/sys_sdl.c:333-458`: the SDL-backed helpers both platform arms
//! share, plus the `counter_freq` / `Sys_DoubleTime` / `Sys_Sleep` pair
//! that `sys_sdl_win.c` and `sys_sdl_unix.c` each spell identically.

use core::ffi::{c_char, c_ulong};
use core::ptr;

use quake_c_sys::quakeflavor_t;
use quake_c_sys::sys as c;

#[cfg(feature = "sdl2")]
use sdl2::sys::{
    SDL_Delay, SDL_GetPerformanceCounter, SDL_GetPerformanceFrequency, SDL_GetPrefPath, SDL_Quit,
    SDL_ShowSimpleMessageBox, SDL_free,
};
#[cfg(feature = "sdl3")]
use sdl3::sys::{
    filesystem::SDL_GetPrefPath,
    init::SDL_Quit,
    messagebox::SDL_ShowSimpleMessageBox,
    stdinc::SDL_free,
    timer::{SDL_Delay, SDL_GetPerformanceCounter, SDL_GetPerformanceFrequency},
};

/// `static double counter_freq;`
static mut COUNTER_FREQ: f64 = 0.0;

/// `counter_freq = (double)SDL_GetPerformanceFrequency ();` (`Sys_Init`).
///
/// # Safety
/// Once, from `Sys_Init`.
pub(super) unsafe fn init_counter_freq() {
    // SAFETY: caller contract -- single-threaded init.
    unsafe { COUNTER_FREQ = SDL_GetPerformanceFrequency() as f64 }
}

/// C: `double Sys_DoubleTime (void)`
///
/// # Safety
/// After `Sys_Init`.
pub unsafe fn double_time() -> f64 {
    // SAFETY: caller contract; the static is only written by `Sys_Init`.
    unsafe { SDL_GetPerformanceCounter() as f64 / COUNTER_FREQ }
}

/// C: `void Sys_Sleep (unsigned long msecs)`
pub fn sleep(msecs: c_ulong) {
    // SAFETY: plain SDL call.
    unsafe { SDL_Delay(msecs as _) }
}

#[cfg(feature = "sdl3")]
mod folder_select {
    use core::ffi::{c_char, c_int, c_void};
    use core::ptr;

    use quake_c_sys::sys as c;
    use sdl3::sys::atomic::{SDL_AtomicInt, SDL_GetAtomicInt, SDL_SetAtomicInt};
    use sdl3::sys::dialog::{
        SDL_ShowFileDialogWithProperties, SDL_FILEDIALOG_OPENFOLDER,
        SDL_PROP_FILE_DIALOG_LOCATION_STRING, SDL_PROP_FILE_DIALOG_TITLE_STRING,
    };
    use sdl3::sys::events::SDL_PumpEvents;
    use sdl3::sys::properties::{
        SDL_CreateProperties, SDL_DestroyProperties, SDL_SetStringProperty,
    };
    use sdl3::sys::timer::SDL_Delay;

    /// `folderselect_t`.
    #[repr(C)]
    struct FolderSelect {
        dst: *mut c_char,
        dstsize: usize,
        done: SDL_AtomicInt,
        result: c_int,
    }

    /// `static void SDLCALL Sys_FolderSelected (void *userdata, const char *const *filelist, int filter)`
    unsafe extern "C" fn folder_selected(
        userdata: *mut c_void,
        filelist: *const *const c_char,
        _filter: c_int,
    ) {
        let sel = userdata.cast::<FolderSelect>();

        // SAFETY: `userdata` is the `FolderSelect` `select_folder` keeps
        // alive until `done` is set; `filelist` is SDL's NULL-terminated
        // array or NULL.
        unsafe {
            if filelist.is_null() {
                (*sel).result = -1; // dialog could not be shown
            } else if (*filelist).is_null() {
                (*sel).result = 0; // cancelled
            } else {
                c::q_strlcpy((*sel).dst, *filelist, (*sel).dstsize);
                (*sel).result = 1;
            }
            SDL_SetAtomicInt(ptr::addr_of_mut!((*sel).done), 1);
        }
    }

    /// C: `int Sys_SelectFolder (const char *title, const char *default_location, char *dst, size_t dstsize)`
    ///
    /// # Safety
    /// `title` is NUL-terminated, `default_location` NUL-terminated or
    /// null, `dst` writable for `dstsize` bytes. Main thread.
    pub unsafe fn select_folder(
        title: *const c_char,
        default_location: *const c_char,
        dst: *mut c_char,
        dstsize: usize,
    ) -> c_int {
        let mut sel = FolderSelect {
            dst,
            dstsize,
            done: SDL_AtomicInt { value: 0 },
            result: 0,
        };

        // SAFETY: caller contract; `sel` outlives the dialog because the
        // loop below spins until the callback flags `done`.
        unsafe {
            let props = SDL_CreateProperties();
            SDL_SetStringProperty(props, SDL_PROP_FILE_DIALOG_TITLE_STRING, title);
            if !default_location.is_null() && *default_location != 0 {
                SDL_SetStringProperty(
                    props,
                    SDL_PROP_FILE_DIALOG_LOCATION_STRING,
                    default_location,
                );
            }
            SDL_ShowFileDialogWithProperties(
                SDL_FILEDIALOG_OPENFOLDER,
                Some(folder_selected),
                ptr::addr_of_mut!(sel).cast(),
                props,
            );
            SDL_DestroyProperties(props);

            // the callback can come from another thread; XDG portals on Linux need event pumping
            while SDL_GetAtomicInt(ptr::addr_of_mut!(sel.done)) == 0 {
                SDL_PumpEvents();
                SDL_Delay(10);
            }
        }

        sel.result
    }
}

#[cfg(feature = "sdl3")]
pub use folder_select::select_folder;

/// C: `char *Sys_GetPrefPath (const char *org, const char *app)`
///
/// # Safety
/// `org`/`app` are NUL-terminated strings.
pub unsafe fn get_pref_path(org: *const c_char, app: *const c_char) -> *mut c_char {
    // SAFETY: caller contract; SDL's buffer is released after the copy.
    unsafe {
        let pref_path = SDL_GetPrefPath(org, app);
        if pref_path.is_null() {
            return ptr::null_mut();
        }
        let result = c::q_strdup(pref_path);
        SDL_free(pref_path.cast());
        result
    }
}

/// C: `void Sys_MessageBoxWarning (const char *title, const char *message)`
///
/// # Safety
/// `title`/`message` are NUL-terminated strings.
pub unsafe fn message_box_warning(title: *const c_char, message: *const c_char) {
    #[cfg(feature = "sdl2")]
    use sdl2::sys::SDL_MessageBoxFlags::SDL_MESSAGEBOX_WARNING;
    #[cfg(feature = "sdl3")]
    use sdl3::sys::messagebox::SDL_MESSAGEBOX_WARNING;

    // SAFETY: caller contract; no parent window.
    unsafe {
        SDL_ShowSimpleMessageBox(SDL_MESSAGEBOX_WARNING as _, title, message, ptr::null_mut());
    }
}

/// C: `FUNC_NORETURN void Sys_QuitNoShutdown (void)`
pub fn quit_no_shutdown() -> ! {
    // SAFETY: plain SDL teardown then the C runtime exit.
    unsafe {
        SDL_Quit();
        c::exit(0)
    }
}

/// C: `quakeflavor_t ChooseQuakeFlavor (void)`
///
/// Shows a simple message box asking the user to choose
/// between the original version and the 2021 rerelease
///
/// # Safety
/// Main thread.
#[cfg(windows)]
pub unsafe fn choose_quake_flavor() -> quakeflavor_t {
    use core::ffi::{c_int, CStr};
    use quake_c_sys::{quakeflavor_t_QUAKE_FLAVOR_ORIGINAL, quakeflavor_t_QUAKE_FLAVOR_REMASTERED};
    #[cfg(feature = "sdl2")]
    use sdl2::sys::{
        SDL_GetError, SDL_MessageBoxButtonData,
        SDL_MessageBoxButtonFlags::SDL_MESSAGEBOX_BUTTON_RETURNKEY_DEFAULT, SDL_MessageBoxData,
        SDL_MessageBoxFlags::SDL_MESSAGEBOX_BUTTONS_LEFT_TO_RIGHT, SDL_ShowMessageBox,
    };
    #[cfg(feature = "sdl3")]
    use sdl3::sys::{
        error::SDL_GetError,
        messagebox::{
            SDL_MessageBoxButtonData, SDL_MessageBoxButtonFlags, SDL_MessageBoxData,
            SDL_ShowMessageBox, SDL_MESSAGEBOX_BUTTONS_LEFT_TO_RIGHT,
            SDL_MESSAGEBOX_BUTTON_RETURNKEY_DEFAULT,
        },
    };

    // native task dialog with command links; requires comctl32 v6
    // with the comctl32 v6 manifest in sys_glue.c, SDL renders this as a native task dialog
    #[cfg(feature = "sdl2")]
    let buttons = [
        SDL_MessageBoxButtonData {
            flags: SDL_MESSAGEBOX_BUTTON_RETURNKEY_DEFAULT as u32,
            buttonid: quakeflavor_t_QUAKE_FLAVOR_REMASTERED as c_int,
            text: c"Remastered".as_ptr(),
        },
        SDL_MessageBoxButtonData {
            flags: 0,
            buttonid: quakeflavor_t_QUAKE_FLAVOR_ORIGINAL as c_int,
            text: c"Original".as_ptr(),
        },
    ];
    #[cfg(feature = "sdl3")]
    let buttons = [
        SDL_MessageBoxButtonData {
            flags: SDL_MESSAGEBOX_BUTTON_RETURNKEY_DEFAULT,
            buttonID: quakeflavor_t_QUAKE_FLAVOR_REMASTERED as c_int,
            text: c"Remastered".as_ptr(),
        },
        SDL_MessageBoxButtonData {
            flags: SDL_MessageBoxButtonFlags(0),
            buttonID: quakeflavor_t_QUAKE_FLAVOR_ORIGINAL as c_int,
            text: c"Original".as_ptr(),
        },
    ];
    let mut choice: c_int = -1;

    let messagebox = SDL_MessageBoxData {
        #[cfg(feature = "sdl2")]
        flags: SDL_MESSAGEBOX_BUTTONS_LEFT_TO_RIGHT as u32,
        #[cfg(feature = "sdl3")]
        flags: SDL_MESSAGEBOX_BUTTONS_LEFT_TO_RIGHT,
        window: ptr::null_mut(),
        title: c"vkqr-engine".as_ptr(),
        message: c"Which Quake version would you like to play?".as_ptr(),
        numbuttons: buttons.len() as c_int,
        buttons: buttons.as_ptr(),
        colorScheme: ptr::null(),
    };

    // SAFETY: caller contract; `messagebox` and `buttons` outlive the call.
    unsafe {
        #[cfg(feature = "sdl3")]
        let failed = !SDL_ShowMessageBox(&messagebox, &mut choice);
        #[cfg(feature = "sdl2")]
        let failed = SDL_ShowMessageBox(&messagebox, &mut choice) < 0;
        if failed {
            super::sys_printf(&format!(
                "ChooseQuakeFlavor: {}\n",
                CStr::from_ptr(SDL_GetError()).to_string_lossy()
            ));
            return quakeflavor_t_QUAKE_FLAVOR_REMASTERED;
        }
    }

    if choice == -1 {
        // SAFETY: plain SDL teardown then the C runtime exit.
        unsafe {
            SDL_Quit();
            c::exit(0);
        }
    }

    choice as quakeflavor_t
}

/// C: `quakeflavor_t ChooseQuakeFlavor (void)` -- the non-Windows arm.
///
/// # Safety
/// Main thread.
#[cfg(not(windows))]
pub unsafe fn choose_quake_flavor() -> quakeflavor_t {
    // FIXME: Original version can't be played on OS's with case-sensitive file systems
    // (due to id1 being named "Id1" and pak0.pak "PAK0.PAK")
    quake_c_sys::quakeflavor_t_QUAKE_FLAVOR_REMASTERED
}
