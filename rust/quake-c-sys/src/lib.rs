//! bindgen externs to remaining C (shrinks to empty)
//!
//! `generated` is the committed output of `scripts/gen_c_bindings.sh`
//! (ADR-011: only this crate declares engine C symbols; CI regenerates and
//! diffs it). `libm` holds hand-written platform libm/CRT declarations with
//! safe wrappers for the `forbid(unsafe_code)` crates (ADR-010).

pub mod libm;

/// Engine globals whose C types cannot be represented portably in the
/// committed bindings (platform-dependent array lengths); only the base
/// address is used from Rust.
pub mod manual {
    use core::ffi::c_char;
    extern "C" {
        /// char com_basedir[MAX_OSPATH] (MAX_OSPATH is PATH_MAX)
        pub static mut com_basedir: [c_char; 0];
    }
}

/// libc stdio over the opaque `FILE` from the generated bindings; the
/// engine's `FS_f*` pak-aware stdio shims wrap these exactly like the C.
pub mod stdio {
    use crate::FILE;
    use core::ffi::{c_char, c_int, c_void};
    extern "C" {
        pub fn fread(ptr: *mut c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
        pub fn fwrite(ptr: *const c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
        pub fn fclose(stream: *mut FILE) -> c_int;
        pub fn fgets(s: *mut c_char, size: c_int, stream: *mut FILE) -> *mut c_char;
        pub fn fgetc(stream: *mut FILE) -> c_int;
        pub fn ferror(stream: *mut FILE) -> c_int;
        pub fn feof(stream: *mut FILE) -> c_int;
        pub fn clearerr(stream: *mut FILE);
    }
}

/// The engine's vendored mimalloc (amalgamated into mem.c's translation unit
/// via `#include "mimalloc/static.c"`; the `mi_*` symbols have external
/// linkage there). Only linked in the Meson mixed build — see quake-capi's
/// `engine-alloc` feature.
pub mod mi {
    use core::ffi::c_void;
    extern "C" {
        /// C: `void *mi_malloc_aligned (size_t size, size_t alignment)`
        pub fn mi_malloc_aligned(size: usize, alignment: usize) -> *mut c_void;
        /// C: `void *mi_zalloc_aligned (size_t size, size_t alignment)`
        pub fn mi_zalloc_aligned(size: usize, alignment: usize) -> *mut c_void;
        /// C: `void *mi_realloc_aligned (void *p, size_t newsize, size_t alignment)`
        pub fn mi_realloc_aligned(p: *mut c_void, newsize: usize, alignment: usize) -> *mut c_void;
        /// C: `void mi_free (void *p)`
        pub fn mi_free(p: *mut c_void);
    }
}

/// Symbols the Phase 7 M2 cvar/command port needs that the committed
/// bindings do not carry: engine string/info helpers, the `sizebuf_t`
/// writers, libc `atof`/`fprintf`, the two registry-owning globals, and the
/// `Quake/cvar_cmd_glue.c` helpers (ADR-009 guarded dispatch + the svs/cls
/// replication blocks that stay C until M6/M7).
pub mod cvar_cmd {
    use crate::{cmd_source_t, cvar_t, cvarcallback_t, qboolean, sizebuf_t, xcommand_t, FILE};
    use core::ffi::{c_char, c_double, c_int, c_uint, c_void};

    extern "C" {
        /* Quake/common.c string helpers */
        pub fn q_strcasecmp(s1: *const c_char, s2: *const c_char) -> c_int;
        pub fn q_strcasestr(haystack: *const c_char, needle: *const c_char) -> *mut c_char;
        pub fn q_strdup(str_: *const c_char) -> *mut c_char;
        /// C: `void Info_SetKey (char *info, size_t infosize, const char *key, const char *val)`
        pub fn Info_SetKey(
            info: *mut c_char,
            infosize: usize,
            key: *const c_char,
            val: *const c_char,
        );
        /// C: `void PR_AutoCvarChanged (cvar_t *var)` -- guarded via CvarCmd_Glue_AutoCvarChanged
        pub fn PR_AutoCvarChanged(var: *mut cvar_t);

        /* Quake/net_msg.c sizebuf primitives (Rust under -Duse_rust_net; the
        C ABI is identical either way) */
        pub fn SZ_Alloc(buf: *mut sizebuf_t, startsize: c_int);
        pub fn SZ_Clear(buf: *mut sizebuf_t);
        pub fn SZ_Write(buf: *mut sizebuf_t, data: *const c_void, length: c_int);

        /* libc */
        pub fn atof(s: *const c_char) -> c_double;
        pub fn fprintf(stream: *mut FILE, fmt: *const c_char, ...) -> c_int;

        /* Quake/host.c */
        pub static mut host_initialized: qboolean;

        /* Quake/cvar_cmd_glue.c data (cmd.c's C-visible globals) */
        pub static mut cmd_text: sizebuf_t;
        pub static mut cmd_source: cmd_source_t;
        pub static mut cl_nopext: cvar_t;
        pub static mut cmd_warncmd: cvar_t;

        /* Quake/cvar_cmd_glue.c helpers -- each returns a Host_Guard status */
        pub fn CvarCmd_Glue_CallXCommand(function: xcommand_t) -> c_int;
        pub fn CvarCmd_Glue_CallCvarCallback(cb: cvarcallback_t, var: *mut cvar_t) -> c_int;
        pub fn CvarCmd_Glue_AutoCvarChanged(var: *mut cvar_t) -> c_int;
        pub fn CvarCmd_Glue_SzWrite(
            buf: *mut sizebuf_t,
            data: *const c_void,
            length: c_int,
        ) -> c_int;
        pub fn CvarCmd_Glue_ServerinfoChanged(var: *mut cvar_t) -> c_int;
        pub fn CvarCmd_Glue_UserinfoChanged(var: *mut cvar_t) -> c_int;
        pub fn CvarCmd_Glue_ForwardBegin() -> c_int;
        pub fn CvarCmd_Glue_ForwardPrint(s: *const c_char) -> c_int;

        /* Quake/cvar_cmd_glue.c accessors for state with no ADR-011 mirror */
        pub fn CvarCmd_Glue_HostClientName() -> *const c_char;
        pub fn CvarCmd_Glue_ClsConnected() -> qboolean;
        pub fn CvarCmd_Glue_ClsDemoPlayback() -> qboolean;
        pub fn CvarCmd_Glue_Protocols(rmq: *mut c_int, fitzquake: *mut c_int, netquake: *mut c_int);
        pub fn CvarCmd_Glue_PextNumbers(
            pext1: *mut c_uint,
            pext1_client: *mut c_uint,
            pext2: *mut c_uint,
            pext2_client: *mut c_uint,
        );
    }
}

#[allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
mod generated;

pub use generated::*;
