//! Phase 7 M6 T6.0: proof that `Quake/sv_user.c`, `Quake/sv_send.c` and
//! `Quake/sv_main.c` really link into the ctest oracle binary.
//!
//! `cargo build -p quake-ctest` compiling those three translation units is not
//! evidence that the M6 port agents can use them: `quake_c_ref.lib` is a static
//! archive, and MSVC pulls a member out of an archive only when something
//! already in the link references one of its symbols. Until some test calls
//! into `sv_user.o` / `sv_send.o` / `sv_main.o`, an unresolved external in any
//! of them stays invisible.
//!
//! So this suite calls one real, non-`static` entry point from each of the
//! three files, through a single C helper in `stubs/stubs.c` (the entry points
//! are renamed `c_ref_*` by `include/c_ref_prelude.h`, so the call has to be
//! spelled on the C side). The three were picked because each returns on its
//! first branch and touches no fixture state:
//!
//! * `SV_ModelIndex` (sv_main.c:824) returns 0 for a null or empty name.
//! * `SV_CreateBaseline` (sv_send.c:2206) loops to `qcvm->num_edicts`, which
//!   the helper sets to 0.
//! * `SV_ClientThink` (sv_user.c:417) returns when `sv_player->v.movetype ==
//!   MOVETYPE_NONE`.
//!
//! What this proves: the three objects link, and their whole transitive
//! unresolved-symbol surface is satisfied by `stubs.c`. What it deliberately
//! does not prove: anything about their behaviour -- that is the M6 port
//! agents' differential suites.

use core::ffi::c_int;
use quake_ctest as _; // links the cc-built c_ref_* archive

extern "C" {
    fn ctest_m6_linkproof() -> c_int;
}

#[test]
fn sv_stratum_objects_link_and_their_entry_points_run() {
    // SAFETY: plain C helper, no arguments, no fixture state touched.
    let mask = unsafe { ctest_m6_linkproof() };

    assert_eq!(
        mask & 1,
        1,
        "SV_ModelIndex (sv_main.c) did not answer 0 for a null/empty name"
    );
    assert_eq!(mask & 2, 2, "SV_CreateBaseline (sv_send.c) did not return");
    assert_eq!(mask & 4, 4, "SV_ClientThink (sv_user.c) did not return");
    assert_eq!(mask, 7, "unexpected extra bits from ctest_m6_linkproof");
}
