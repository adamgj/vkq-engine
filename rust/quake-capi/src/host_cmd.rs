//! `Quake/host_cmd.c` -- the console command surface, the map/mod/demo/save
//! file lists and the savegame reader/writer (Rust migration Phase 7 M8,
//! T8.3).
//!
//! Compiled instead of `host_cmd.c` under `-Duse_rust_host`, the same
//! Pattern A whole-file swap `host.c` took at T8.2. `Quake/host_cmd_glue.c`
//! owns the C-visible data and the ADR-009 guarded seams; this module owns
//! every behavioural body.
//!
//! ADR-007: no dual-view row opens or closes here. All seven C-visible data
//! objects `host_cmd.c` defined -- `current_skill`, `noclip_anglehack`,
//! `extralevels`, `extralevels_sorted`, `modlist`, `demolist`, `savelist` --
//! keep C storage in the glue, because `pr_edict.c`, `sv_main.c`,
//! `cl_parse.c`, `in_sdl.c`, `view.c`, `console.c` and `menu.c` all resolve
//! them by name, and three of those translation units are already Rust
//! (`sv_main.rs`, `cl_parse.rs`, `view.rs`) with `extern "C"` declarations
//! that would collide with a Rust definition -- the M5 defect MSVC merges
//! silently and Linux hard-errors. Only the three file-`static`s
//! (`maxlevelnamelen`, `extralevels_parsing_thread`,
//! `extralevels_cancel_parsing`) and `RightPad`'s function-scope buffer had
//! internal linkage; of those, the thread handle and the atomic stay C
//! because `atomics.h`'s accessors are `static inline` with
//! compiler-specific barriers that must not be re-derived in Rust.
//!
//! ADR-016: `ExtraMaps_ParseDescriptions` is the body of a `QThread_Create`
//! worker. It stays reachable as a plain `extern "C"` function pointer and
//! touches no Rust thread-local, no ambient qcvm and no `Host_Guard` frame;
//! the thread creation, join and cancel flag remain C in the glue.
//!
//! ADR-005: `Host_Savegame_f` and `Host_Loadgame_f` are byte-diff subjects.
//! Neither body contains a `%g` or `%e` specifier -- the five `%g` a
//! savegame carries are formatted inside `gl_fog.c` and `gl_sky.c`, which
//! are out of Phase 7 scope; this module calls `Fog_GetFogCommand` and
//! `Sky_GetSkyCommand` through capi and emits the returned string opaquely
//! through `%s`, never re-deriving those numbers.

#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_c_sys::host_cmd as g;
use quake_types::host::{Client, ClientState, ClientStatic, FileListItem, Server, ServerStatic};

use crate::cl_main::{cl, cls};
use crate::sv_main::{sv, svs};

// ---------------------------------------------------------------------------
// ADR-009 plumbing. Identical to `host.rs`: a `Host_Guard` status of 0 =
// returned normally, 1 = `Host_Error` / `Host_EndGame`, 2 = `screen_error`.

/// A `Host_Guard` status.
pub(crate) type Raise = c_int;

macro_rules! raise {
    ($e:expr) => {{
        let r: Raise = $e;
        if r != 0 {
            return r;
        }
    }};
}

// ---------------------------------------------------------------------------
// shared accessors, mirroring `host.rs:178-256`.

#[inline]
fn sv_p() -> *mut Server {
    ptr::addr_of_mut!(sv).cast::<Server>()
}

#[inline]
fn svs_p() -> *mut ServerStatic {
    ptr::addr_of_mut!(svs).cast::<ServerStatic>()
}

#[inline]
fn cl_p() -> *mut ClientState {
    ptr::addr_of_mut!(cl).cast::<ClientState>()
}

#[inline]
fn cls_p() -> *mut ClientStatic {
    ptr::addr_of_mut!(cls).cast::<ClientStatic>()
}

// `host.c`'s `host_client`, which `host_cmd.c` reads and writes. Typed with
// the ADR-011 mirror, so it is declared here rather than in `quake-c-sys`
// (the `sv_main.rs:103` / `sv_send.rs:121` precedent).
extern "C" {
    static mut host_client: *mut Client;
}

#[inline]
unsafe fn host_client_get() -> *mut Client {
    // SAFETY: engine global, single-threaded.
    unsafe { ptr::addr_of!(host_client).read() }
}

#[inline]
unsafe fn host_client_set(v: *mut Client) {
    // SAFETY: engine global, single-threaded.
    unsafe { ptr::addr_of_mut!(host_client).write(v) }
}

/// `cvar_t::value` without taking a reference to the C-owned storage.
#[inline]
unsafe fn cvar_value(var: *const c::cvar_t) -> c_float {
    // SAFETY: caller passes a live engine cvar.
    unsafe { ptr::addr_of!((*var).value).read() }
}
