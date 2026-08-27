//! The savegame writer: `PR_UglyValueString`, `ED_Write`, `ED_WriteGlobals`
//! (`pr_edict.c`).
//!
//! This is the subject of ADR-019's gate 2 — `scripts/harness/save_diff.py`
//! byte-compares the `.sav` two builds produce — so the output here is
//! byte-exact or nothing. Every number goes through `quake_util::printf`
//! rather than Rust's `format!`, whose shortest-roundtrip floats would differ
//! (ADR-005).
//!
//! **Specifier audit (ROADMAP.md:42):** the writers moved here use `%s`, `%f`,
//! `%i`, `%u`, `%PRIi64` and `%PRIu64` only. `pr_edict.c` contains no `%g` or
//! `%e` anywhere, so the two conversions `quake_util::printf` deliberately
//! leaves unimplemented are unreachable from this phase.

use core::ffi::c_int;

use quake_types::progs::{etype, DDef, DEF_SAVEGLOBAL};
use quake_util::printf::{format, Arg};

use crate::arena::{StringError, VmRaw};

/// `protocol.h`: `ENTALPHA_DEFAULT` is 0 so zeroed memory means "default".
pub const ENTALPHA_DEFAULT: u8 = 0;
/// `protocol.h`: fully transparent.
pub const ENTALPHA_ZERO: u8 = 1;

/// `protocol.h` `ENTALPHA_TOSAVE`.
///
/// COMPAT: the divisor is `254` and the arithmetic is done in `float`, so the
/// value written into a savegame is the single-precision quotient.
#[must_use]
pub fn entalpha_tosave(a: u8) -> f32 {
    if a == ENTALPHA_DEFAULT {
        0.0
    } else if a == ENTALPHA_ZERO {
        -1.0
    } else {
        (f32::from(a) - 1.0) / 254.0
    }
}

/// `progs.h` `NUM_TYPE_SIZES`
pub const NUM_TYPE_SIZES: c_int = 8;

/// `pr_edict.c` `type_size[]`. Note it only covers the eight vanilla types
/// (`NUM_TYPE_SIZES`); the DP/FTE extended types are not in the table.
#[must_use]
pub fn type_size(t: c_int) -> c_int {
    match t {
        etype::EV_VECTOR => 3,
        _ => 1,
    }
}

/// How many 32-bit words a value of `t` actually occupies.
///
/// Unlike [`type_size`] this covers the extended types: the `Q_ALIGN(4)`
/// 64-bit ones (`pr_comp.h`) take two words. Used to size the exact
/// destination a writer or parser may touch — an over-wide slice would reach
/// past the globals block or the last edict.
#[must_use]
pub fn value_words(t: c_int) -> usize {
    match t & !c_int::from(DEF_SAVEGLOBAL) {
        etype::EV_VECTOR => 3,
        etype::EV_EXT_DOUBLE | etype::EV_EXT_SINT64 | etype::EV_EXT_UINT64 => 2,
        _ => 1,
    }
}

/// What the writer needs from code that has not moved yet.
pub trait SaveSys {
    /// `ED_FieldAtOfs` — the linear search over `fielddefs` for a field whose
    /// offset matches, used by `PR_UglyValueString`'s `ev_field` arm.
    fn field_at_ofs(&mut self, ofs: c_int) -> Option<DDef>;
}

/// The errors the writer can hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveError {
    /// A string handle whose slot was cleared.
    NonExistentString(c_int),
    /// `PR_UglyValueString`'s `ev_entity` arm reached `NUM_FOR_EDICT` with a
    /// prog offset outside `[0, num_edicts)`.
    BadEdictPointer,
}

fn resolve(vm: &VmRaw, handle: c_int) -> Result<&[u8], SaveError> {
    vm.get_string_bytes(handle)
        .map_err(|StringError::NonExistent(n)| SaveError::NonExistentString(n))
}

/// `pr_edict.c`: `PR_UglyValueString` formats into a `static char line[1024]`
/// with `q_snprintf`, so its result is capped at 1023 bytes plus the NUL.
/// That cap is part of the function's observable contract — `ED_Write` writes
/// whatever survives it — so it is applied here rather than only at the
/// exported shim.
const UGLY_VALUE_MAX: usize = 1024 - 1;

/// `PR_UglyValueString` — the savegame-facing value formatter.
///
/// `val` is the raw words at the value's location; only as many as the type
/// needs are read.
///
/// COMPAT: the result is truncated to [`UGLY_VALUE_MAX`] bytes, because C's
/// `q_snprintf` into `line[1024]` truncates. Only the `ev_string` arm can
/// reach the cap in practice (a mod storing a >1 KB string in an entity
/// field); every other arm formats a fixed-width number.
pub fn ugly_value_string(
    vm: &VmRaw,
    sys: &mut dyn SaveSys,
    ty: c_int,
    val: &[i32],
) -> Result<Vec<u8>, SaveError> {
    let ty = ty & !c_int::from(DEF_SAVEGLOBAL);
    let w0 = val.first().copied().unwrap_or(0);

    let out = match ty {
        etype::EV_STRING => format(b"%s", &[Arg::Str(resolve(vm, w0)?)]),
        etype::EV_ENTITY => {
            // NUM_FOR_EDICT (PROG_TO_EDICT (val->edict)). C's NUM_FOR_EDICT
            // raises `Host_Error ("NUM_FOR_EDICT: bad pointer")` in *release*
            // builds when the resulting index falls outside
            // `[0, num_edicts)`, so the range test is part of the writer's
            // observable behaviour, not a debug-only assertion.
            let num = w0 / vm.edict_stride().max(1);
            if num < 0 || num >= vm.num_edicts() {
                return Err(SaveError::BadEdictPointer);
            }
            format(b"%i", &[Arg::I32(num)])
        }
        etype::EV_FUNCTION => {
            // COMPAT (accepted divergence, same rationale as `ev_field`
            // below): C computes `qcvm->functions + val->function` with no
            // bounds check and dereferences it, so an out-of-range function
            // reference reads past the lump. The port writes an empty name
            // instead. Unreachable from a well-formed progs.
            let name = vm
                .function_name_handle(w0)
                .map_or(Ok(&b""[..]), |h| resolve(vm, h))?;
            format(b"%s", &[Arg::Str(name)])
        }
        etype::EV_FIELD => {
            let def = sys.field_at_ofs(w0);
            // COMPAT: C dereferences ED_FieldAtOfs's result without a null
            // check. A savegame reaching that is already crashing in C; an
            // empty name is the closest non-crashing behaviour.
            let name = match def {
                Some(d) => resolve(vm, d.s_name)?,
                None => &b""[..],
            };
            format(b"%s", &[Arg::Str(name)])
        }
        etype::EV_VOID => b"void".to_vec(),
        etype::EV_FLOAT => format(b"%f", &[Arg::F64(f64::from(f32::from_bits(w0 as u32)))]),
        etype::EV_EXT_INTEGER => format(b"%i", &[Arg::I32(w0)]),
        etype::EV_EXT_UINT32 => format(b"%u", &[Arg::U32(w0 as u32)]),
        etype::EV_EXT_SINT64 => format(b"%lli", &[Arg::I64(read_i64(val))]),
        etype::EV_EXT_UINT64 => format(b"%llu", &[Arg::U64(read_i64(val) as u64)]),
        etype::EV_EXT_DOUBLE => format(b"%f", &[Arg::F64(f64::from_bits(read_i64(val) as u64))]),
        etype::EV_VECTOR => {
            let f = |i: usize| f64::from(f32::from_bits(val.get(i).copied().unwrap_or(0) as u32));
            format(
                b"%f %f %f",
                &[Arg::F64(f(0)), Arg::F64(f(1)), Arg::F64(f(2))],
            )
        }
        other => format(b"bad type %i", &[Arg::I32(other)]),
    };
    Ok(truncate_to_c_buffer(out))
}

/// Applies `PR_UglyValueString`'s `line[1024]` cap.
fn truncate_to_c_buffer(mut out: Vec<u8>) -> Vec<u8> {
    out.truncate(UGLY_VALUE_MAX);
    out
}

/// The 64-bit QC types are `Q_ALIGN(4)` (`pr_comp.h`), so they are read as two
/// words rather than as an aligned `i64`.
fn read_i64(val: &[i32]) -> i64 {
    let lo = val.first().copied().unwrap_or(0) as u32 as u64;
    let hi = val.get(1).copied().unwrap_or(0) as u32 as u64;
    (lo | (hi << 32)) as i64
}

/// True when `ED_Write`/`ED_Print` would skip this field name as one of a
/// vector's synthesized `_x`/`_y`/`_z` components.
fn is_vector_component(name: &[u8]) -> bool {
    name.len() > 1 && name[name.len() - 2] == b'_'
}

/// `ED_Write`. Appends to `out` exactly what C `fprintf`s.
pub fn ed_write(
    vm: &VmRaw,
    sys: &mut dyn SaveSys,
    num: c_int,
    out: &mut Vec<u8>,
) -> Result<(), SaveError> {
    if vm.edict_free(num) {
        out.extend_from_slice(b"{\n}\n");
        return Ok(());
    }

    out.extend_from_slice(b"{\n");

    // field 0 is skipped: the loop starts at 1, as in C
    for i in 1..vm.numfielddefs() {
        let d = vm.fielddef(i);
        let ty = c_int::from(d.type_);
        if ty & c_int::from(DEF_SAVEGLOBAL) != 0 {
            // saved by ED_WriteGlobals instead
            continue;
        }
        if ty >= NUM_TYPE_SIZES {
            continue;
        }

        let name = resolve(vm, d.s_name)?;
        if is_vector_component(name) {
            continue;
        }

        let count = type_size(ty) as usize;
        // COMPAT (accepted divergence): C computes
        // `(eval_t *)((char *)&ed->v + d->ofs * 4)` and reads it with no
        // bounds check, so a fielddef whose offset runs past `entityfields`
        // reads adjacent memory and writes whatever it finds. The port cannot
        // reproduce an out-of-bounds read, and inventing a raise C does not
        // have would abort saves C completes, so the field is omitted. Both
        // behaviours are unreachable from a well-formed progs, where
        // `entityfields` covers every fielddef offset by construction.
        let Some(words) = vm.edict_field_words(num, c_int::from(d.ofs), count) else {
            continue;
        };

        // if the value is still all 0, skip the field
        if words.iter().all(|&w| w == 0) {
            continue;
        }

        let value = ugly_value_string(vm, sys, c_int::from(d.type_), &words)?;
        out.extend_from_slice(&format(
            b"\"%s\" \"%s\"\n",
            &[Arg::Str(name), Arg::Str(&value)],
        ));
    }

    // johnfitz -- save entity alpha manually when progs.dat doesn't know
    // about alpha
    let alpha = vm.edict_alpha(num);
    if vm.extfield_alpha() < 0 && alpha != ENTALPHA_DEFAULT {
        out.extend_from_slice(&format(
            b"\"alpha\" \"%f\"\n",
            &[Arg::F64(f64::from(entalpha_tosave(alpha)))],
        ));
    }

    out.extend_from_slice(b"}\n");
    Ok(())
}

/// `ED_WriteGlobals`.
pub fn ed_write_globals(
    vm: &VmRaw,
    sys: &mut dyn SaveSys,
    out: &mut Vec<u8>,
) -> Result<(), SaveError> {
    out.extend_from_slice(b"{\n");
    for i in 0..vm.numglobaldefs() {
        let def = vm.globaldef(i);
        let raw = c_int::from(def.type_);
        if raw & c_int::from(DEF_SAVEGLOBAL) == 0 {
            continue;
        }
        let ty = raw & !c_int::from(DEF_SAVEGLOBAL);

        if !matches!(
            ty,
            etype::EV_STRING
                | etype::EV_FLOAT
                | etype::EV_EXT_DOUBLE
                | etype::EV_EXT_INTEGER
                | etype::EV_EXT_UINT32
                | etype::EV_EXT_SINT64
                | etype::EV_EXT_UINT64
                | etype::EV_ENTITY
        ) {
            continue;
        }

        let name = resolve(vm, def.s_name)?;
        out.extend_from_slice(&format(b"\"%s\" ", &[Arg::Str(name)]));

        // C passes the *masked* type here, unlike ED_Write which passes the
        // raw one; both end up masking, but the distinction is preserved.
        // Read exactly the words the type occupies: a wider read would run
        // past the globals block for a def at its tail.
        let words = vm.g_words(def.ofs as usize, value_words(ty));
        let value = ugly_value_string(vm, sys, ty, &words)?;
        out.extend_from_slice(&format(b"\"%s\"\n", &[Arg::Str(&value)]));
    }
    out.extend_from_slice(b"}\n");
    Ok(())
}
