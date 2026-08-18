//! Differential test: `quake_util::qctype` vs the static-inline originals in
//! `Quake/q_ctype.h` (wrapped as `c_ref_*` by stubs/qctype_ref.c).
//!
//! The C functions take and return `int`, so the whole argument range that
//! callers can produce is swept — including the negative values a signed
//! `char` yields for bytes >= 0x80, which is exactly where a naive port
//! (`u8`-based, or Rust's `is_ascii_*`) would diverge.
// The c_ref_* symbols are compiled C (build.rs), which Miri cannot execute;
// the shims themselves get Miri coverage in miri_capi.rs instead.
#![cfg(not(miri))]

use core::ffi::c_int;
use quake_ctest as _; // links the cc-built c_ref_* archive
use quake_util::qctype;

extern "C" {
    fn c_ref_q_isascii(c: c_int) -> c_int;
    fn c_ref_q_islower(c: c_int) -> c_int;
    fn c_ref_q_isupper(c: c_int) -> c_int;
    fn c_ref_q_isalpha(c: c_int) -> c_int;
    fn c_ref_q_isdigit(c: c_int) -> c_int;
    fn c_ref_q_isxdigit(c: c_int) -> c_int;
    fn c_ref_q_isalnum(c: c_int) -> c_int;
    fn c_ref_q_isblank(c: c_int) -> c_int;
    fn c_ref_q_isspace(c: c_int) -> c_int;
    fn c_ref_q_isgraph(c: c_int) -> c_int;
    fn c_ref_q_isprint(c: c_int) -> c_int;
    fn c_ref_q_toascii(c: c_int) -> c_int;
    fn c_ref_q_tolower(c: c_int) -> c_int;
    fn c_ref_q_toupper(c: c_int) -> c_int;
}

#[test]
fn all_predicates_match_c() {
    // every byte value, both sign extensions of a char, and the boundaries
    let ranges = [-256i32..=512, i32::MIN..=i32::MIN, i32::MAX..=i32::MAX];
    for range in ranges {
        for c in range {
            // SAFETY: pure value-in/value-out C functions
            unsafe {
                assert_eq!(
                    qctype::q_isascii(c),
                    c_ref_q_isascii(c) != 0,
                    "q_isascii({c})"
                );
                assert_eq!(
                    qctype::q_islower(c),
                    c_ref_q_islower(c) != 0,
                    "q_islower({c})"
                );
                assert_eq!(
                    qctype::q_isupper(c),
                    c_ref_q_isupper(c) != 0,
                    "q_isupper({c})"
                );
                assert_eq!(
                    qctype::q_isalpha(c),
                    c_ref_q_isalpha(c) != 0,
                    "q_isalpha({c})"
                );
                assert_eq!(
                    qctype::q_isdigit(c),
                    c_ref_q_isdigit(c) != 0,
                    "q_isdigit({c})"
                );
                assert_eq!(
                    qctype::q_isxdigit(c),
                    c_ref_q_isxdigit(c) != 0,
                    "q_isxdigit({c})"
                );
                assert_eq!(
                    qctype::q_isalnum(c),
                    c_ref_q_isalnum(c) != 0,
                    "q_isalnum({c})"
                );
                assert_eq!(
                    qctype::q_isblank(c),
                    c_ref_q_isblank(c) != 0,
                    "q_isblank({c})"
                );
                assert_eq!(
                    qctype::q_isspace(c),
                    c_ref_q_isspace(c) != 0,
                    "q_isspace({c})"
                );
                assert_eq!(
                    qctype::q_isgraph(c),
                    c_ref_q_isgraph(c) != 0,
                    "q_isgraph({c})"
                );
                assert_eq!(
                    qctype::q_isprint(c),
                    c_ref_q_isprint(c) != 0,
                    "q_isprint({c})"
                );
                assert_eq!(qctype::q_toascii(c), c_ref_q_toascii(c), "q_toascii({c})");
                assert_eq!(qctype::q_tolower(c), c_ref_q_tolower(c), "q_tolower({c})");
                assert_eq!(qctype::q_toupper(c), c_ref_q_toupper(c), "q_toupper({c})");
            }
        }
    }
}
