//! `Quake/pr_edict_arena_glue.c` declarations (Rust migration Phase 7 M9d).
//!
//! ADR-011: engine C symbols are declared only in this crate.
//!
//! The arena flip needs exactly one glue seam. Everything else it reaches --
//! `Mem_Alloc`/`Mem_Realloc`/`Mem_Free`, `Con_Warning`, `Con_DPrintf2` and
//! `PRParse_Glue_UnlinkEdict` (`Quake/pr_edict_parse_glue.c:81`, compiled
//! under the same `-Duse_rust_progs`) -- is already in the committed
//! bindings, and `ED_ALLOC_HOOK` never crosses the boundary at all: the hook
//! stays a C function pointer that `pr_edict_arena_glue.c` stores and calls,
//! reached from Rust only through `ED_AllocSetHook` (declared in
//! `crate::sv_phys`).

use core::ffi::c_int;

extern "C" {
    /// `Quake/pr_edict_arena_glue.c`: `qsort (nums, n, sizeof (int),
    /// ED_freetime_compare_func)`, the free-list rebuild's sort.
    ///
    /// COMPAT (ADR-010): `ED_freetime_compare_func` is `(int)copysign (1.0,
    /// a - b)`, which is never 0, so the comparator is inconsistent and
    /// `qsort`'s tie ordering is implementation-defined. Entity numbering is
    /// observable (savegames, the wire protocol), so the *same* platform
    /// `qsort` and the *same* comparator have to keep deciding it -- no Rust
    /// sort may substitute. See `quake_progs::alloc::freetime_compare`, whose
    /// own COMPAT note is why `rebuild_free_list` takes the sort as a
    /// parameter.
    ///
    /// The comparator dereferences each number with `EDICT_NUM_NO_CHECK`;
    /// callers must have range-checked them against `qcvm->max_edicts`
    /// already (`quake-capi`'s shim does, before it can reach this).
    pub fn PREdictArena_Glue_SortFreeEdicts(nums: *mut c_int, n: usize);
}
