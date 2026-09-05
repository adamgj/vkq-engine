//! Phase 7 M7 T7.0: proof that `Quake/chase.c`, `Quake/cl_demo.c`,
//! `Quake/cl_input.c`, `Quake/cl_main.c`, `Quake/cl_parse.c`,
//! `Quake/cl_tent.c` and `Quake/view.c` really link into the ctest oracle
//! binary.
//!
//! Same reasoning as `sv_stratum_linkproof.rs` (M6): `quake_c_ref.lib` is a
//! static archive, so a green `cargo build -p quake-ctest --tests` only proves
//! the seven translation units *compiled*. Until something references a symbol
//! each of them defines, an unresolved external inside one stays invisible.
//!
//! So this suite calls one real, non-`static` entry point per file, through a
//! single C helper in `stubs/stubs.c` (the entry points are renamed `c_ref_*`
//! by `include/c_ref_prelude.h`, so the calls have to be spelled on the C
//! side). Each was picked to be observable and to touch as little fixture
//! state as possible:
//!
//! * `Chase_Init` (chase.c:36) registers the four chase cvars, which is what
//!   populates `chase_back.value` / `chase_up.value` from their default
//!   strings.
//! * `CL_Stop_f` (cl_demo.c) takes the "Not recording a demo." early return.
//! * `CL_KeyState` (cl_input.c) is pure: state 1 (held, no impulses) is 1.0.
//! * `CL_AllocDlight` (cl_main.c) hands back a slot stamped with the key.
//! * `CL_EntityNum` (cl_parse.c) grows `cl.num_entities` up to the requested
//!   index and returns that slot.
//! * `CL_UpdateTEnts` (cl_tent.c) unconditionally resets `num_temp_entities`
//!   before walking the (empty) beam list.
//! * `V_CalcRoll` (view.c) rolls -2 degrees for velocity straight along the
//!   entity's right vector at the default `cl_rollangle`.
//!
//! What this proves: the seven objects link, and their whole transitive
//! unresolved-symbol surface is satisfied by `stubs.c`. What it deliberately
//! does not prove: anything about their behaviour -- that is the job of the
//! later M7 differential suites.

use core::ffi::c_int;
use quake_ctest as _; // links the cc-built c_ref_* archive

extern "C" {
    fn ctest_m7_linkproof() -> c_int;
}

#[test]
fn cl_stratum_objects_link_and_their_entry_points_run() {
    // SAFETY: plain C helper, no arguments; it saves and restores the `cl`
    // fields it borrows.
    let mask = unsafe { ctest_m7_linkproof() };

    assert_eq!(
        mask & 1,
        1,
        "Chase_Init (chase.c) did not register chase_back/chase_up"
    );
    assert_eq!(mask & 2, 2, "CL_Stop_f (cl_demo.c) did not return");
    assert_eq!(
        mask & 4,
        4,
        "CL_KeyState (cl_input.c) did not answer 1.0 for a held key"
    );
    assert_eq!(
        mask & 8,
        8,
        "CL_AllocDlight (cl_main.c) did not stamp the requested key"
    );
    assert_eq!(
        mask & 16,
        16,
        "CL_EntityNum (cl_parse.c) did not grow cl.num_entities to the index"
    );
    assert_eq!(
        mask & 32,
        32,
        "CL_UpdateTEnts (cl_tent.c) did not reset num_temp_entities"
    );
    assert_eq!(
        mask & 64,
        64,
        "V_CalcRoll (view.c) did not roll -2 degrees at the default cl_rollangle"
    );
    assert_eq!(mask, 127, "unexpected extra bits from ctest_m7_linkproof");
}
