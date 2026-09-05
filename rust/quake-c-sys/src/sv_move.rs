//! `Quake/sv_move_glue.c` declarations (Rust migration Phase 7 M4).
//!
//! ADR-011: engine C symbols are declared only in this crate. The glue file
//! owns every `Host_Guard` call site reachable from `quake-capi`'s `sv_move`
//! module, so no `longjmp` unwinds a Rust frame (ADR-009).
//!
//! `SV_CheckBottom`/`SV_movestep`/`SV_StepDirection`/`SV_NewChaseDir`/
//! `SV_MoveToGoal` are `Quake/sv_move_glue.c`'s five re-raising wrappers over
//! this module's `quake_rs_sv_*` cores; nothing in `quake-capi` calls them
//! (ADR-009), so they are not declared here. `World_Glue_AssertFailed`
//! (`Quake/world_glue.c`) already covers this file's three `assert_always`
//! sites and is declared in `quake_c_sys::world`.

extern "C" {
    /// C: `void PF_changeyaw (void)` (`Quake/pr_cmds.c`), called directly by
    /// `SV_StepDirection` (`sv_move.c:242`). Under `USE_RUST_PROGS` this is a
    /// leaf builtin (`quake_rs_pf_changeyaw`, see
    /// `rust/quake-capi/src/progs_builtins.rs`) that never reaches
    /// `PR_ExecuteProgram`, so it cannot `Host_Error` and needs no guard.
    pub fn PF_changeyaw();
}
