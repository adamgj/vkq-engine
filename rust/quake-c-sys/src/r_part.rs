//! `Quake/r_part_glue.c` declarations (Rust migration Phase 7 M10f-1).
//!
//! ADR-011: engine C symbols are declared only in this crate. `r_part.c` is a
//! Pattern A whole-file swap that is split rather than moved wholesale: the
//! simulation half becomes `quake-capi/src/r_part.rs` and the rendering half
//! stays C in `Quake/r_part_glue.c` (it is Vulkan-typed throughout and belongs
//! to Phase 8 per `ROADMAP.md`). The seam below is what the two halves share.
//!
//! ## Why the pool stays C
//!
//! ADR-007: `active_particles`, `free_particles`, `particles` and
//! `r_numparticles` are read by the rendering half that stays in C --
//! `R_DrawParticlesFaces` (`r_part.c:962`, `:984`, `:997`) walks the active
//! list, and `R_InitParticleIndexBuffer` (`r_part.c:161`, `:206`) sizes the
//! index buffer from `r_numparticles`. Keeping the storage in the glue is the
//! same call `Quake/sbar_glue.c` makes for `fragsort`/`scoreboardlines`, and
//! it keeps the port's pool a genuinely separate object from the oracle's in
//! the differential build. Rust reaches it through the externs below and an
//! ADR-011 [`particle_t`] mirror.
//!
//! `r_particles`, `r_quadparticles`, `particletexture*`, `texturescalefactor`
//! and `particle_index_buffer` stay C for the same reason -- every one of them
//! is read by the rendering half. `ramp1`/`ramp2`/`ramp3` (`r_part.c:34-36`)
//! are read only by the simulation, so they moved to Rust.
//!
//! `COM_Rand`, `Mem_Alloc`, `COM_CheckParm`, `com_argv`, `Con_Printf`,
//! `COM_FOpenFile` and `Cvar_SetCallback` come from the generated bindings and
//! `fclose` from the crate-root `stdio` block, so none of them is re-declared
//! here; `atoi` is not in either, so it is declared below the
//! way every other module that needs them does.
//!
//! `cl` and `cls` are mirror-typed and are reached through `quake-capi`
//! (`crate::cl_main` owns the storage since T7.4); this crate has no
//! `[dependencies]` and cannot name `quake-types`.

use crate::cvar_t;
use crate::FILE;
use core::ffi::{c_char, c_float, c_int, c_uint, c_void};

/// `r_part.c:266` -- `#define NUMVERTEXNORMALS 162`.
pub const NUMVERTEXNORMALS: usize = 162;

/// `glquake.h:81-91` -- `ptype_t`. Eight non-negative enumerators, so the
/// implementation type is `int` on every platform the engine targets;
/// `Harness_HashParticles` (`r_part.c:945`) hashes `sizeof (p->type)` bytes of
/// it, which makes the width part of the state-hash contract.
#[allow(non_camel_case_types)]
pub type ptype_t = c_int;

/// `glquake.h:83`
pub const PT_STATIC: ptype_t = 0;
/// `glquake.h:84`
pub const PT_GRAV: ptype_t = 1;
/// `glquake.h:85`
pub const PT_SLOWGRAV: ptype_t = 2;
/// `glquake.h:86`
pub const PT_FIRE: ptype_t = 3;
/// `glquake.h:87`
pub const PT_EXPLODE: ptype_t = 4;
/// `glquake.h:88`
pub const PT_EXPLODE2: ptype_t = 5;
/// `glquake.h:89`
pub const PT_BLOB: ptype_t = 6;
/// `glquake.h:90`
pub const PT_BLOB2: ptype_t = 7;

/// ADR-011 mirror of `particle_t` (`glquake.h:94-105`). bindgen never sees it:
/// every prototype that mentions it also mentions `cb_context_t`, which the
/// core headers deliberately keep out of the bindgen surface. The layout is
/// transcribed in full -- `R_ClearParticles` (`r_part.c:337-341`) walks the
/// pool by pointer arithmetic and `Harness_HashParticles` recovers the pool
/// index the same way, so `size_of` has to be right.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct particle_t {
    // driver-usable fields
    pub org: [c_float; 3],
    pub color: c_float,
    // drivers never touch the following fields
    pub next: *mut particle_t,
    pub vel: [c_float; 3],
    pub ramp: c_float,
    pub die: c_float,
    pub type_: ptype_t,
}

extern "C" {
    /* ---------------------------------------------------------------------
     * Glue-owned storage (r_part.c:38, :42, :47-48).
     */

    /// `r_part.c:38` -- head of the live list, walked by
    /// `R_DrawParticlesFaces`.
    pub static mut active_particles: *mut particle_t;
    /// `r_part.c:38` -- head of the LIFO free list.
    pub static mut free_particles: *mut particle_t;
    /// `r_part.c:38` -- base of the `Mem_Alloc`'d pool; NULL until
    /// `R_InitParticles` runs, which is what makes `Harness_HashParticles` a
    /// no-op on a dedicated server.
    pub static mut particles: *mut particle_t;
    /// `r_part.c:42` -- pool length, read by `R_InitParticleIndexBuffer`.
    pub static mut r_numparticles: c_int;

    /// `r_part.c:47` -- `cvar_t r_particles`. Declared in no header;
    /// `menu.c:119` and `r_part.c:960` re-declare it locally.
    pub static mut r_particles: cvar_t;
    /// `r_part.c:48` -- `cvar_t r_quadparticles`, read only by
    /// `R_DrawParticlesFaces`.
    pub static mut r_quadparticles: cvar_t;

    /// `r_alias.c:37` -- `float r_avertexnormals[NUMVERTEXNORMALS][3]`, the
    /// shared anorms table `R_EntityParticles` (`r_part.c:317-319`) spreads
    /// its particles over. Read-only here.
    pub static r_avertexnormals: [[c_float; 3]; NUMVERTEXNORMALS];

    /* ---------------------------------------------------------------------
     * Glue entry points.
     */

    /// `Quake/r_part_glue.c` -- `R_InitParticles`' rendering tail
    /// (`r_part.c:250-254`), i.e. `if (!no_rendering) { R_InitParticleTextures
    /// (); R_InitParticleIndexBuffer (); }`. Both callees are Vulkan-typed and
    /// stay C.
    pub fn RPart_Glue_InitRender();

    /// `Quake/r_part_glue.c` -- `Host_Guard` trampoline over
    /// `Cvar_RegisterVariable` (`r_part.c:247`, `:249`), which is itself a
    /// `Host_Reraise` wrapper under `-Duse_rust_cvar` (ADR-009 rule 3).
    pub fn RPart_Glue_RegisterVariable(var: *mut cvar_t) -> c_int;

    /// `Quake/r_part_glue.c` -- external-linkage wrapper over `r_part.c:135`'s
    /// `static R_SetParticleTexture_f`, so the Rust core can hand
    /// `Cvar_SetCallback` (`r_part.c:248`) a stable C function pointer for a
    /// callback that lives in the rendering half.
    pub fn RPart_Glue_SetParticleTexture_f(var: *mut cvar_t);

    /* ---------------------------------------------------------------------
     * Everything else the simulation half reaches.
     */

    /// `common.h` -- `float MSG_ReadCoord (unsigned int flags)`.
    pub fn MSG_ReadCoord(flags: c_uint) -> c_float;
    /// `common.h` -- `int MSG_ReadChar (void)`.
    pub fn MSG_ReadChar() -> c_int;
    /// `common.h` -- `int MSG_ReadByte (void)`.
    pub fn MSG_ReadByte() -> c_int;

    /// `harness.h:106` -- the FNV-style state-hash accumulator
    /// `Harness_HashParticles` folds each live particle into.
    pub fn Harness_Hash64(h: u64, data: *const c_void, len: usize) -> u64;

    /// `common.c:633` -- `int q_snprintf (char *, size_t, const char *, ...)`.
    pub fn q_snprintf(str_: *mut c_char, size: usize, format: *const c_char, ...) -> c_int;

    /// libc `int atoi (const char *)` -- `R_InitParticles` (`r_part.c:234`).
    pub fn atoi(s: *const c_char) -> c_int;
    /// `r_part_glue.c` -- `int RPart_Glue_ScanPoint (FILE *, vec3_t)`, the
    /// glue's shim over `fscanf (f, "%f %f %f\n", ...)` (`r_part.c:376`).
    /// A direct `fscanf` extern here compiled and linked under `cargo test`
    /// but failed the meson/clang-cl engine link with `LNK2019: unresolved
    /// external symbol fscanf` (M10f-1 integration). The mechanism was not
    /// established -- the pre-existing `fscanf` externs in `menu.rs` and
    /// `cl_demo.rs` resolve in the same binary, so it is not that `fscanf`
    /// lacks an importable symbol. Routing through the glue removes the
    /// dependency and keeps libc's exact scanner, which is the compat
    /// surface -- the pointfile is plain text and reimplementing float
    /// parsing in Rust would be a new divergence rather than a port.
    pub fn RPart_Glue_ScanPoint(stream: *mut FILE, org: *mut c_float) -> c_int;
}
