/* pr_edict_arena_glue_ref.c -- ctest-link mirror of the one Quake/
 * pr_edict_arena_glue.c seam quake-capi's arena shim imports (Rust migration
 * Phase 7 M9d, T9.3).
 *
 * Quake/pr_edict_arena_glue.c is a Meson-only translation unit: build.rs
 * compiles Quake/pr_edict_arena.c (C_SOURCES) as the c_ref oracle, never the
 * glue. Task-plan lesson (gg) -- every Pattern A flip needs its *_glue_ref.c
 * mirror -- so the single symbol quake-capi/src/pr_edict_arena.rs declares
 * (in quake-c-sys/src/pr_edict_arena.rs) is defined here, or enabling the
 * feature would break the link with no compile-time warning (lesson (ff):
 * `cargo clippy --all-targets` type-checks but never links).
 *
 * Scope: exactly one symbol. Everything else quake-capi's arena shim reaches
 * already resolves in this link -- Mem_Alloc/Mem_Realloc/Mem_Free (mem.c),
 * Con_Warning/Con_DPrintf2 (stubs.c), PRParse_Glue_UnlinkEdict (stubs.c:6793),
 * qcvm (stubs.c:3148) and nullentitystate (protocol.c's, not renamed).
 * ED_Alloc/ED_Free/PR_GetString and the rest of the flipped file's public
 * entry points are deliberately NOT defined here: c_ref_prelude.h:609-619
 * renames them to c_ref_*, and the pr_edict_arena.c oracle already defines
 * those. Defining them again under either spelling would be a duplicate.
 *
 * Currently dead in this link: quake-ctest's Cargo.toml does not enable
 * quake-capi's `progs` feature, so quake-capi::pr_edict_arena is not compiled
 * into the test binaries at all. The mirror is here so it stays correct if
 * that changes; the module's only compile gate today is CI's
 * `cargo clippy -p quake-capi --features progs` job.
 *
 * Differential status (the sv_user_ref.c:170-182 / sv_main_ref.c:124-148
 * question): tests/progs_arena_differential.rs DOES exist for this file, but
 * the two #undef exceptions do not apply and there is no vacuity risk. Those
 * two exist solely to stop a *renamed oracle symbol* being short-circuited
 * back into the Rust side. PREdictArena_Glue_SortFreeEdicts is a new seam
 * name the prelude does not rename, ED_freetime_compare_func below is a file
 * static with no linkage to collide, and the differential does not route
 * through either: it drives quake_progs::alloc::rebuild_free_list with
 * stubs.c's ctest_progs_sort_by_freetime (progs_arena_differential.rs:214-230)
 * against c_ref_ED_RebuildFreeList. Nothing in this link calls the function
 * below, so it cannot make any comparison vacuous.
 *
 * Callee spelling follows the host_cmd_glue_ref.c rule: every callee is
 * spelled the way Quake/pr_edict_arena_glue.c spells it and there is no
 * #undef anywhere in this file, so c_ref_prelude.h's per-TU renames rewrite
 * both sides identically. EDICT_NUM_NO_CHECK is a progs.h macro over the
 * unrenamed `qcvm` (prelude:605-608 explains why qcvm is deliberately left
 * alone), and qsort/copysign come from the prelude's <stdlib.h>/<math.h>.
 */

/* Verbatim from Quake/pr_edict_arena.c:181-186, as the glue keeps it. ADR-010:
 * (int)copysign (1.0, a - b) is never 0, so the comparator is inconsistent and
 * qsort's tie ordering is implementation-defined; entity numbering is
 * observable, so the same platform qsort and the same comparator must keep
 * deciding it. The unchecked EDICT_NUM form is used because a Host_Error out
 * of a comparator would longjmp through libc; callers range-check first. */
static int ED_freetime_compare_func (const void *first, const void *second)
{
	int firstInt = *(const int *)first;
	int secondInt = *(const int *)second;
	return (int)copysign (1.0, EDICT_NUM_NO_CHECK (firstInt)->freetime - EDICT_NUM_NO_CHECK (secondInt)->freetime);
}

void PREdictArena_Glue_SortFreeEdicts (int *nums, size_t n)
{
	qsort (nums, n, sizeof (int), ED_freetime_compare_func);
}
